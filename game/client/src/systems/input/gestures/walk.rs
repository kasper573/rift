use std::time::Duration;

use bevy::prelude::*;
use bevy::window::CursorIcon;
use world::core::math::Pos;
use world::core::tiling::{TilePos, Tiles};
use world::systems::player::session;
use world::systems::{area, combat};

use crate::core::render;
use crate::systems::input::gestures::{ActiveTileHighlight, Gesture, image_cursor};

const MOVE_REPEAT: Duration = Duration::from_millis(333);

pub struct WalkGesture;

inventory::submit! {
    &WalkGesture as &dyn Gesture
}

#[derive(Resource, Default)]
struct WalkState {
    last_tile: Option<Pos<Tiles>>,
    last_sent: Option<Duration>,
}

impl Gesture for WalkGesture {
    fn priority(&self) -> i32 {
        3
    }

    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world) && target(world).is_some()
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
        let held = world
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left);
        let path = if held {
            "icons/cursors/pointer011.png"
        } else {
            "icons/cursors/pointer010.png"
        };
        let handle = world.resource::<AssetServer>().load(path);
        Some(image_cursor(handle, (32, 32)))
    }

    fn tile_highlight(&self, world: &mut World) -> Option<ActiveTileHighlight> {
        let pos = target(world)?;
        let image = world
            .resource::<AssetServer>()
            .load("icons/crosshairs/white/crosshair026.png");
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
