//! On a client, the ground around the player's own character is held: what the client predicts
//! walks on, and what it checks a click against.

use std::collections::BTreeSet;

use bevy::tasks::futures::check_ready;
use bevy::tasks::{IoTaskPool, Task};
use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::prelude::*;

use super::held_reach;
use crate::core::content::Content;
use crate::core::ground::TileAt;
use crate::core::units::Meters;
use crate::systems::area::GroundHold;
use crate::systems::movement::Position;
use crate::systems::player::session::my_character;
use crate::systems::{area, sleep};

/// A hold fetches and lays ground, so it runs on a task of its own, one at a time, and the frames
/// go on meanwhile.
pub struct ClientRegionPlugin;

impl Plugin for ClientRegionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, follow);
    }
}

#[derive(Resource)]
struct Holding {
    reach: Meters,
    held: Option<GroundHold>,
    task: Option<Task<GroundHold>>,
}

fn follow(world: &mut World) {
    let Some(content) = world.get_resource::<Content>().cloned() else {
        return;
    };
    let Some((me, at)) = my_character(world).and_then(|me| {
        let at = me.get::<Position>()?.pos;
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
    if !world.contains_resource::<Holding>() {
        world.insert_resource(Holding {
            reach: held_reach(sleep::awake_reach(&content)),
            held: None,
            task: None,
        });
    }
    let mut holding = world.resource_mut::<Holding>();
    if let Some(task) = holding.task.as_mut() {
        let Some(held) = check_ready(task) else {
            return;
        };
        holding.task = None;
        holding.held = Some(held);
    }
    let wanted: BTreeSet<TileAt> = area.tiles_around(at, holding.reach).collect();
    let around = holding
        .held
        .as_ref()
        .is_some_and(|held| held.holds_on(&area) && *held.tiles() == wanted);
    if around {
        return;
    }
    let previous = holding.held.take();
    holding.task =
        Some(IoTaskPool::get().spawn(async move { area.hold_instead(previous, wanted) }));
}
