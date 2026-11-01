//! The ground near players is all a world holds of its area: the tiles of ground within reach of
//! a player are loaded and their meshes laid as the player comes near, and what the area's scene
//! places on a tile comes to life the first time the tile is reached. A land costs only what its
//! players have come to, however much of it there is.

use std::collections::{BTreeSet, HashSet};
use std::thread::JoinHandle;

use bevy::tasks::futures_lite::future;
use bevy::tasks::{IoTaskPool, Task, block_on};
use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::prelude::*;

use crate::core::chunk::{CHUNK_SIDE, Chunk};
use crate::core::content::Content;
use crate::core::ground::TileAt;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::player::session::my_character;
use crate::systems::sleep::Sleep;
use crate::systems::visibility::VIEW_DISTANCE;
use crate::systems::{WorldArea, area, npc, script, sleep};

/// How far around a player the ground is held, which tiles this world has brought to life, and
/// the hold under way.
#[derive(Resource)]
pub struct Region {
    reach: Meters,
    reached: HashSet<TileAt>,
    holding: Option<JoinHandle<BTreeSet<TileAt>>>,
    /// The chunks players stood in when the ground around them was last asked for.
    stood: Vec<Chunk>,
}

impl Region {
    /// Held is as far as anything awake stands: the chunks awake reach past a player by as far as
    /// a client sees and anything there reaches, rounded up to whole chunks.
    pub fn of(content: &Content) -> Region {
        Region {
            reach: Meters(VIEW_DISTANCE.0 + sleep::acting_reach(content).0 + CHUNK_SIDE.0 * 2.0),
            reached: HashSet::new(),
            holding: None,
            stood: Vec::new(),
        }
    }
}

/// Holds the ground within reach of every player, on a thread of its own so the tick goes on
/// while tiles load and their meshes are laid, and brings to life what stands on tiles reached
/// for the first time once they are held. Players walk far slower than ground is held, and the
/// ground is held as far past them as anything awake stands, so they never reach its edge. Runs
/// after the sleep settles, and asks for ground only when a player stands in a new chunk.
pub fn settle(world: &mut World) {
    let finished = world
        .resource::<Region>()
        .holding
        .as_ref()
        .is_some_and(JoinHandle::is_finished);
    if finished {
        let held = world
            .resource_mut::<Region>()
            .holding
            .take()
            .expect("the hold under way")
            .join()
            .expect("a hold finishes");
        come_to_life(world, held);
    }
    let region = world.resource::<Region>();
    if region.holding.is_some() || region.stood == world.resource::<Sleep>().stood() {
        return;
    }
    let stood = world.resource::<Sleep>().stood().to_vec();
    world.resource_mut::<Region>().stood = stood.clone();
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let area = area::load(&content, area_id);
    let region = world.resource::<Region>();
    let wanted: BTreeSet<TileAt> = stood
        .into_iter()
        .flat_map(|chunk| area.tiles_around(chunk.rect().center(), region.reach))
        .filter(|tile| !region.reached.contains(tile))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let area = area::loaded(&content, area_id)
        .expect("the area loaded")
        .clone();
    world.resource_mut::<Region>().holding = Some(std::thread::spawn(move || {
        area.hold(wanted.iter().copied());
        wanted
    }));
}

/// Whether the ground within reach of the world's players is held, with no hold under way.
pub fn settled(world: &World) -> bool {
    world.resource::<Region>().holding.is_none()
}

/// Holds the ground within reach of `at` at once, before something arrives there: a player
/// joining, an actor warped or carried in.
pub fn reach(world: &mut World, at: Pos<Meters>) {
    let content = world.resource::<Content>().clone();
    let area = area::load(&content, world.resource::<WorldArea>().0);
    let reach = world.resource::<Region>().reach;
    let wanted: BTreeSet<TileAt> = area.tiles_around(at, reach).collect();
    area.hold(wanted.iter().copied());
    come_to_life(world, wanted);
}

/// Holds every tile of the area and brings all of it to life: what a world small enough to hold
/// whole asks for, as a test's does.
pub fn reach_all(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let area = area::load(&content, world.resource::<WorldArea>().0);
    let wanted: BTreeSet<TileAt> = area.ground_size().each_tile().collect();
    area.hold(wanted.iter().copied());
    come_to_life(world, wanted);
}

/// Brings to life what the scene places on each of `held`, those not reached before: its
/// placements' scripts, its objects and its npcs.
fn come_to_life(world: &mut World, held: BTreeSet<TileAt>) {
    let fresh: Vec<TileAt> = {
        let mut region = world.resource_mut::<Region>();
        held.into_iter()
            .filter(|tile| region.reached.insert(*tile))
            .collect()
    };
    for tile in fresh {
        script::spawn_placed(world, tile);
        area::object::spawn_in(world, tile);
        npc::spawn_in(world, tile);
    }
}

/// On a client, the ground around the player's own character is held: what the client predicts
/// walks on, and what it checks a click against. A hold fetches and lays ground, so it runs on a
/// task of its own, one at a time, and the frames go on meanwhile.
pub struct ClientRegionPlugin;

impl Plugin for ClientRegionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Holding>()
            .add_systems(PreUpdate, follow);
    }
}

#[derive(Resource, Default)]
struct Holding(Option<Task<()>>);

fn follow(world: &mut World) {
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let Some((me, at)) = my_character(world).and_then(|me| {
        let at = me.get::<crate::systems::movement::Position>()?.pos;
        Some((me.id(), at))
    }) else {
        return;
    };
    let Some(area_id) = world.get::<area::AreaTag>(me).map(|tag| tag.area) else {
        return;
    };
    let Some(area) = area::loaded(&content, area_id).cloned() else {
        return;
    };
    if !world.contains_resource::<Region>() {
        let region = Region::of(&content);
        world.insert_resource(region);
    }
    let reach = world.resource::<Region>().reach;
    let mut holding = world.resource_mut::<Holding>();
    if let Some(task) = holding.0.as_mut() {
        if block_on(future::poll_once(task)).is_none() {
            return;
        }
        holding.0 = None;
    }
    let held = area.held();
    let wanted: Vec<TileAt> = area
        .tiles_around(at, reach)
        .filter(|tile| !held.contains(tile))
        .collect();
    if wanted.is_empty() {
        return;
    }
    holding.0 = Some(IoTaskPool::get().spawn(async move {
        area.hold(wanted);
    }));
}
