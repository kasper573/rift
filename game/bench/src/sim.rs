use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, ServerState};
use bevy_state::prelude::NextState;
use strum::VariantArray;
use world::core::assets::AssetService;
use world::core::math::Pos;
use world::core::tiling::Tiles;
use world::data;
use world::systems::area::{self, Area};
use world::systems::item::Inventory;
use world::systems::npc::Npc;
use world::systems::player::{ClientId, Owner, Players, Xp};
use world::systems::visibility::OwnedBy;

pub const NPCS_PER_AREA: usize = 25;
pub const PLAYERS_PER_AREA: usize = 25;

pub type Roster = Vec<(ClientId, Entity)>;

/// The `areas` highest-numbered benchmark area worlds, each populated with NPCs and players, plus
/// each world's roster of (client, player). Shared by the benchmark and the profiler so both
/// measure the exact same simulation.
pub fn worlds(areas: usize, npc: data::npc::Id, assets: &AssetService) -> (Vec<App>, Vec<Roster>) {
    let mut worlds = Vec::with_capacity(areas);
    let mut rosters = Vec::with_capacity(areas);
    for id in data::area::Id::VARIANTS
        .iter()
        .copied()
        .filter(|id| id.get().bench)
        .take(areas)
    {
        let (app, roster) = build_world(id, npc, assets);
        worlds.push(app);
        rosters.push(roster);
    }
    (worlds, rosters)
}

/// Connects every rostered player as a client, giving each its owner-scoped replicated view.
pub fn connect(worlds: &mut [App], rosters: &[Roster]) {
    for (app, roster) in worlds.iter_mut().zip(rosters) {
        let world = app.world_mut();
        for &(client, player) in roster {
            world.spawn((ConnectedClient { max_size: 1200 }, client));
            world.entity_mut(player).insert(OwnedBy(client));
        }
    }
}

/// Advances every world one tick.
pub fn step(worlds: &mut [App]) {
    for app in worlds.iter_mut() {
        app.update();
    }
}

fn build_world(id: area::Id, npc: data::npc::Id, assets: &AssetService) -> (App, Roster) {
    let mut app = world::systems::server_app(id);
    app.insert_resource(assets.clone());
    app.insert_resource(world::core::math::Rng::from_entropy());
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update();

    let area = assets.resolve(id.get().map, area::build_area);
    let world = app.world_mut();
    let content: Vec<Entity> = world
        .query_filtered::<Entity, With<Npc>>()
        .iter(world)
        .collect();
    for entity in content {
        world.despawn(entity);
    }

    for _ in 0..NPCS_PER_AREA {
        let entity = spawn_character(world, npc, wander_pos(area), id);
        world.entity_mut(entity).insert(Npc {
            def: npc,
            group: id.index() as u32,
        });
    }
    let mut roster = Vec::with_capacity(PLAYERS_PER_AREA);
    for index in 0..PLAYERS_PER_AREA {
        let client = ClientId(index as u32 + 1);
        let player = spawn_character(world, npc, player_spot(area, index), id);
        world
            .entity_mut(player)
            .insert((Owner { client }, Inventory::empty(), Xp { amount: 0 }));
        world.resource_mut::<Players>().0.insert(client, player);
        roster.push((client, player));
    }
    (app, roster)
}

/// Spreads players evenly across the area's walkable cells (by striding the walkable-node list) so
/// connected clients have realistically distinct areas-of-interest, rather than all stacking on the
/// spawn tile and sharing one identical view.
fn player_spot(area: &Area, index: usize) -> Pos<Tiles> {
    let nodes = &area.walkable_nodes;
    if nodes.is_empty() {
        return area.spawn;
    }
    nodes[index * nodes.len() / PLAYERS_PER_AREA]
}

fn wander_pos(area: &Area) -> Pos<Tiles> {
    area.walkable_nodes.first().copied().unwrap_or(area.spawn)
}

fn spawn_character(
    world: &mut World,
    def_id: data::npc::Id,
    at: Pos<Tiles>,
    area: area::Id,
) -> Entity {
    world::systems::npc::spawn_actor(world, def_id.get(), at, area)
}
