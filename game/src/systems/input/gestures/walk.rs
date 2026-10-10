use std::time::Duration;

use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::player::session;
use crate::systems::{area, combat};
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::core::content::{Content, Fixture};
use crate::core::render;
use crate::systems::input::gestures::{ActiveTileHighlight, Gesture, content_cursor};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::{CursorShape, InterfaceIcon};

const MOVE_REPEAT: Duration = Duration::from_millis(333);

pub struct WalkGesture;

#[derive(Resource, Default)]
struct WalkState {
    last_tile: Option<Pos<Tiles>>,
    last_sent: Option<Duration>,
}

impl Gesture for WalkGesture {
    fn priority(&self) -> i32 {
        5
    }

    fn claims(&self, world: &mut World) -> bool {
        session::is_alive(world) && target(world).is_some()
    }

    fn drive(&self, world: &mut World, start: bool) {
        if !start {
            repeat(world);
            return;
        }
        if let Some(tile) = target(world) {
            session::move_to(world, tile);
            stamp(world, Some(tile));
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        target(world)?;
        let shape = if map::pressed(world, InputAction::Interact) {
            CursorShape::WalkHeld
        } else {
            CursorShape::Walk
        };
        Some(content_cursor(world, shape))
    }

    fn tile_highlight(&self, world: &mut World) -> Option<ActiveTileHighlight> {
        let pos = target(world)?;
        let target = InterfaceIcon::WalkTarget
            .get(world.resource::<Content>())
            .image;
        let image = world.resource::<AssetServer>().load(target.0);
        Some(ActiveTileHighlight { pos, image })
    }
}

fn target(world: &mut World) -> Option<Pos<Tiles>> {
    let tile = render::cursor_tile(world)?.snap();
    area::walkable(world, tile).then_some(tile)
}

fn repeat(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    let last_sent = world.get_resource::<WalkState>().and_then(|s| s.last_sent);
    if last_sent.is_some_and(|sent| now.saturating_sub(sent) < MOVE_REPEAT) {
        return;
    }
    let Some(point) = render::cursor_tile(world) else {
        return;
    };
    if combat::enemy_at(world, point).is_some() {
        return;
    }
    let tile = point.snap();
    let last_tile = world.get_resource::<WalkState>().and_then(|s| s.last_tile);
    if !area::walkable(world, tile) || last_tile == Some(tile) {
        return;
    }
    session::move_to(world, tile);
    stamp(world, Some(tile));
}

fn stamp(world: &mut World, tile: Option<Pos<Tiles>>) {
    let now = world.resource::<Time>().elapsed();
    let mut state = world.get_resource_or_insert_with(WalkState::default);
    state.last_sent = Some(now);
    state.last_tile = tile;
}
