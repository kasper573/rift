//! On a client, the ground around the player's own character is held: what the client predicts
//! walks on, and what it checks a click against.

use bevy::tasks::futures::check_ready;
use bevy::tasks::{IoTaskPool, Task};
use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::prelude::*;

use super::held_reach;
use crate::core::content::Content;
use crate::core::ground::TileAt;
use crate::core::units::Meters;
use crate::systems::area;
use crate::systems::movement::Position;
use crate::systems::player::session::my_character;

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
    task: Option<Task<()>>,
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
            reach: held_reach(&content),
            task: None,
        });
    }
    let mut holding = world.resource_mut::<Holding>();
    if let Some(task) = holding.task.as_mut() {
        if check_ready(task).is_none() {
            return;
        }
        holding.task = None;
    }
    let held = area.held();
    let wanted: Vec<TileAt> = area
        .tiles_around(at, holding.reach)
        .filter(|tile| !held.contains(tile))
        .collect();
    if wanted.is_empty() {
        return;
    }
    holding.task = Some(IoTaskPool::get().spawn(async move {
        area.hold(wanted);
    }));
}
