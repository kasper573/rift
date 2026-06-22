use std::time::Duration;

use bevy::prelude::*;
use world::math::Pos;
use world::query;
use world::session;
use world::tiling::{TilePos, Tiles};

use crate::gestures::InputIntent;
use crate::render;

const MOVE_REPEAT: Duration = Duration::from_millis(333);

#[derive(Resource, Default)]
pub(super) struct HeldMove {
    last_tile: Option<Pos<Tiles>>,
    last_sent: Option<Duration>,
}

pub(super) struct Walk;

impl InputIntent for Walk {
    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world) && render::cursor_tile(world).is_some()
    }

    fn drive(&self, world: &mut World, start: bool) {
        if !start {
            repeat_move(world);
            return;
        }
        if let Some(point) = render::cursor_tile(world) {
            session::move_to(world, point);
            stamp(world, Some(point.snap()));
        }
    }
}

fn repeat_move(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    if world
        .resource::<HeldMove>()
        .last_sent
        .is_some_and(|sent| now.saturating_sub(sent) < MOVE_REPEAT)
    {
        return;
    }
    let Some(point) = render::cursor_tile(world) else {
        return;
    };
    if query::enemy_at(world, point).is_some() {
        return;
    }
    let tile = point.snap();
    if !query::walkable(world, tile) || world.resource::<HeldMove>().last_tile == Some(tile) {
        return;
    }
    session::move_to(world, point);
    stamp(world, Some(tile));
}

fn stamp(world: &mut World, tile: Option<Pos<Tiles>>) {
    let now = world.resource::<Time>().elapsed();
    let mut state = world.resource_mut::<HeldMove>();
    state.last_tile = tile;
    state.last_sent = Some(now);
}
