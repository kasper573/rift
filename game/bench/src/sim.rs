use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, ConnectedClient, FromClient, ServerState};
use bevy_state::prelude::NextState;
use world::core::assets::AssetService;
use world::data;
use world::systems::player::{ClientId, JoinRequest, SpawnPolicy};

pub const PLAYERS_PER_AREA: usize = 25;

/// The connection entity per player (one per simulated client). Their characters are spawned by the
/// world's own `join` system; [`connect`] later promotes them to fully replicating clients. Shared
/// by the benchmark and the profiler so both measure the exact same simulation.
pub type Roster = Vec<Entity>;

/// How the area's players are positioned, which sets how much their areas-of-interest overlap. Maps
/// directly onto the server's [`SpawnPolicy`], so the bench spreads players exactly as production does.
#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    /// Every player spawns on the spawn tile, so all clients share one identical area-of-interest —
    /// the worst case for replicating nearby entities to many overlapping viewers.
    Congested,
    /// Players spawn across the area's walkable cells, so clients have realistically distinct
    /// areas-of-interest.
    Distributed,
}

impl Layout {
    pub fn label(self) -> &'static str {
        match self {
            Layout::Congested => "congested",
            Layout::Distributed => "distributed",
        }
    }

    fn spawn_policy(self) -> SpawnPolicy {
        match self {
            Layout::Congested => SpawnPolicy::Map,
            Layout::Distributed => SpawnPolicy::Dist,
        }
    }
}

/// The NPCs each bench area spawns, read from the content layer (the same area the bench instances).
pub fn npcs_per_area() -> usize {
    data::area::BENCH_ID
        .get()
        .spawns
        .iter()
        .map(|spawn| spawn.population as usize)
        .sum()
}

/// Exactly `areas` instances of the benchmark area, each populated with its NPCs and players, plus
/// each world's roster of connection entities. The area count is a pure runtime parameter — every
/// world is an instance of the one area template, so the count built always equals the count asked for.
pub fn worlds(areas: usize, layout: Layout, assets: &AssetService) -> (Vec<App>, Vec<Roster>) {
    let mut worlds = Vec::with_capacity(areas);
    let mut rosters = Vec::with_capacity(areas);
    for ordinal in 0..areas {
        let (app, roster) = build_world(layout, assets, ordinal as u64);
        worlds.push(app);
        rosters.push(roster);
    }
    (worlds, rosters)
}

/// Promotes every rostered connection into a fully replicating client, giving each its owner-scoped
/// replicated view.
pub fn connect(worlds: &mut [App], rosters: &[Roster]) {
    for (app, roster) in worlds.iter_mut().zip(rosters) {
        let world = app.world_mut();
        for &conn in roster {
            world
                .entity_mut(conn)
                .insert(ConnectedClient { max_size: 1200 });
        }
    }
}

/// Advances every world one tick across all cores — the same parallel area fan-out the real server
/// uses, so the benchmark's capacity reflects the threaded server.
pub fn step(worlds: &mut [App]) {
    world::systems::step_areas(worlds);
}

/// Advances every world one tick on the calling thread only. Used by the profiler so CPU samples
/// attribute cleanly to game systems instead of being scattered across rayon worker frames.
#[cfg(feature = "profiling")]
pub fn step_single_threaded(worlds: &mut [App]) {
    for app in worlds.iter_mut() {
        app.update();
    }
}

fn build_world(layout: Layout, assets: &AssetService, ordinal: u64) -> (App, Roster) {
    let mut app = world::systems::server_app(world::data::area::BENCH_ID, ordinal);
    app.insert_resource(assets.clone());
    app.insert_resource(world::core::math::Rng::from_entropy());
    app.insert_resource(layout.spawn_policy());
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update(); // Startup `npc::spawn_all` populates the area's real NPC distribution.

    let world = app.world_mut();
    let mut roster = Vec::with_capacity(PLAYERS_PER_AREA);
    for index in 0..PLAYERS_PER_AREA {
        let conn = world.spawn(ClientId(index as u32 + 1)).id();
        world.write_message(FromClient {
            client_id: Sender::Client(conn),
            message: JoinRequest,
        });
        roster.push(conn);
    }
    app.update(); // the real `player::join` system spawns each player's character.
    (app, roster)
}
