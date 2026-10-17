use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::ConnectedClient;
use bevy_replicon::shared::backend::server_messages::ServerMessages;
use engine::core::content::Content;
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::systems::area::AreaModule;
use engine::systems::player::{ClientId, JoinRequest, SpawnPolicy, SpawnRules};
use engine::systems::{GameSettings, TICK_HZ, from_client, server_app, start_server, step_areas};
use strum::AsRefStr;

use crate::play;

pub const PLAYERS_PER_AREA: usize = 25;

const CLOCK: WallClock = WallClock {
    now: UnixMillis(0),
    reset: UtcHour::MIDNIGHT,
};

/// The wall clock ticks with the worlds, so cooldowns and played actions run their course as they
/// do on the live server.
fn clock_after(ticks: usize) -> WallClock {
    WallClock {
        now: CLOCK.now.after(Seconds(ticks as f32 / TICK_HZ.0)),
        ..CLOCK
    }
}

/// Ticks stepped before sampling, so spawn waves and connect churn settle out of the measurement.
pub const WARMUP: usize = 30;

// Runs play out alike, so they compare.
const SEED: u64 = 1931;

pub type Roster = Vec<Entity>;

/// How the area's players are positioned, which sets how much their areas-of-interest overlap. Maps
/// directly onto the server's [`SpawnPolicy`], so the bench spreads players exactly as production does.
#[derive(Clone, Copy, PartialEq, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum Layout {
    /// Every player spawns on the spawn tile, so all clients share one identical area-of-interest —
    /// the worst case for replicating nearby entities to many overlapping viewers.
    Congested,
    /// Players spawn across the area's walkable cells, so clients have realistically distinct
    /// areas-of-interest.
    Distributed,
}

impl Layout {
    fn spawn_policy(self) -> SpawnPolicy {
        match self {
            Layout::Congested => SpawnPolicy::Map,
            Layout::Distributed => SpawnPolicy::Dist,
        }
    }
}

pub fn npcs_per_area(content: &Content) -> usize {
    let area = content
        .built::<GameSettings>()
        .initial_spawn
        .area
        .get(content);
    area.modules
        .iter()
        .map(|module| match module {
            AreaModule::Population(population) => population.count as usize,
            AreaModule::Resident(_) => 1,
            _ => 0,
        })
        .sum()
}

pub fn worlds(areas: usize, layout: Layout, content: &Content) -> (Vec<App>, Vec<Roster>) {
    let mut worlds = Vec::with_capacity(areas);
    let mut rosters = Vec::with_capacity(areas);
    for ordinal in 0..areas {
        let (app, roster) = build_world(layout, content, ordinal as u64);
        worlds.push(app);
        rosters.push(roster);
    }
    (worlds, rosters)
}

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
/// uses, so the benchmark's capacity reflects the threaded server — after its players have made
/// their moves.
pub fn step(worlds: &mut [App], rosters: &[Roster], content: &Content) {
    let ticks = TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    for (app, roster) in worlds.iter_mut().zip(rosters) {
        play::act(app.world_mut(), content, roster, ticks);
    }
    step_areas(worlds, clock_after(ticks));
    consume_sent(worlds);
}

static TICKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Advances every world one tick on the calling thread only. Used by the profiler so CPU samples
/// attribute cleanly to game systems instead of being scattered across rayon worker frames.
#[cfg(feature = "profiling")]
pub fn step_single_threaded(worlds: &mut [App]) {
    let ticks = TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    for app in worlds.iter_mut() {
        app.insert_resource(clock_after(ticks));
        app.update();
    }
    consume_sent(worlds);
}

/// What the server's transport does with outgoing messages once they are on the wire.
fn consume_sent(worlds: &mut [App]) {
    for app in worlds.iter_mut() {
        app.world_mut()
            .resource_mut::<ServerMessages>()
            .drain_sent()
            .for_each(drop);
    }
}

fn build_world(layout: Layout, content: &Content, ordinal: u64) -> (App, Roster) {
    let mut app = server_app(
        content.built::<GameSettings>().initial_spawn.area,
        ordinal,
        SEED,
        CLOCK,
        content,
    );
    app.insert_resource(SpawnRules {
        policy: layout.spawn_policy(),
        immortal: false,
    });
    start_server(&mut app);

    let zone = content.built::<GameSettings>().initial_spawn.area;
    let world = app.world_mut();
    let mut roster = Vec::with_capacity(PLAYERS_PER_AREA);
    for index in 0..PLAYERS_PER_AREA {
        let conn = world.spawn(ClientId(index as u32 + 1)).id();
        play::join_as(world, content, zone, conn, index);
        from_client(world, conn, JoinRequest);
        roster.push(conn);
    }
    app.update(); // the real `player::join` system spawns each player's character.
    (app, roster)
}
