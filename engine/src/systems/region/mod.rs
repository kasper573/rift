//! The ground near players is all a world holds of its area: the tiles of ground within reach of
//! a player are loaded and their meshes laid as the player comes near, and what the area's scene
//! places on a tile comes to life the first time the tile is reached. A land costs only what its
//! players have come to, however much of it there is.

use std::collections::{BTreeSet, HashSet};
use std::thread::JoinHandle;

pub use client::ClientRegionPlugin;

use bevy_ecs::prelude::*;

use crate::core::chunk::{CHUNK_SIDE, Chunk};
use crate::core::content::Content;
use crate::core::ground::TileAt;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::sleep::Sleep;
use crate::systems::{GameSettings, WorldArea, area, npc, script, sleep};

mod client;

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
    pub fn of(content: &Content) -> Region {
        Region {
            reach: held_reach(content),
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
pub fn hold_around(world: &mut World, at: Pos<Meters>) {
    let content = world.resource::<Content>().clone();
    let area = area::load(&content, world.resource::<WorldArea>().0);
    let reach = world.resource::<Region>().reach;
    let wanted: BTreeSet<TileAt> = area.tiles_around(at, reach).collect();
    area.hold(wanted.iter().copied());
    come_to_life(world, wanted);
}

/// Holds every tile of the area and brings all of it to life: what a world small enough to hold
/// whole asks for, as a test's does.
pub fn hold_whole(world: &mut World) {
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

/// How far around a player ground is held: as far as anything awake stands, the chunks awake
/// reaching past a player by as far as a client sees and anything there reaches, rounded up to
/// whole chunks.
fn held_reach(content: &Content) -> Meters {
    let sight = content.built::<GameSettings>().sight;
    Meters(sight.0 + sleep::acting_reach(content).0 + CHUNK_SIDE.0 * 2.0)
}
