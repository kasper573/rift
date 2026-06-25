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

/// Walk toward the cursor's tile: move once on press, then re-issue while held as the cursor moves.
pub struct WalkGesture {
    walk: Handle<Image>,
    walk_held: Handle<Image>,
    highlight: Handle<Image>,
    last_tile: Option<Pos<Tiles>>,
    last_sent: Option<Duration>,
}

impl WalkGesture {
    pub fn new(assets: &AssetServer) -> WalkGesture {
        WalkGesture {
            walk: assets.load("icons/cursors/pointer010.png"),
            walk_held: assets.load("icons/cursors/pointer011.png"),
            highlight: assets.load("icons/crosshairs/white/crosshair026.png"),
            last_tile: None,
            last_sent: None,
        }
    }

    /// The walkable tile under the cursor — where a press moves to, what the cursor marks, and where the
    /// crosshair sits. `None` when the cursor is off-map or over a blocked tile.
    pub fn target(&self, world: &mut World) -> Option<Pos<Tiles>> {
        let tile = render::cursor_tile(world)?.snap();
        area::walkable(world, tile).then_some(tile)
    }

    fn repeat(&mut self, world: &mut World) {
        let now = world.resource::<Time>().elapsed();
        if self
            .last_sent
            .is_some_and(|sent| now.saturating_sub(sent) < MOVE_REPEAT)
        {
            return;
        }
        let Some(point) = render::cursor_tile(world) else {
            return;
        };
        if combat::enemy_at(world, point).is_some() {
            return;
        }
        let tile = point.snap();
        if !area::walkable(world, tile) || self.last_tile == Some(tile) {
            return;
        }
        session::move_to(world, tile);
        self.stamp(world, Some(tile));
    }

    fn stamp(&mut self, world: &mut World, tile: Option<Pos<Tiles>>) {
        self.last_sent = Some(world.resource::<Time>().elapsed());
        self.last_tile = tile;
    }
}

impl Gesture for WalkGesture {
    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world) && self.target(world).is_some()
    }

    fn drive(&mut self, world: &mut World, start: bool) {
        if !start {
            self.repeat(world);
            return;
        }
        if let Some(tile) = self.target(world) {
            session::move_to(world, tile);
            self.stamp(world, Some(tile));
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        self.target(world)?;
        let held = world
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left);
        let handle = if held {
            self.walk_held.clone()
        } else {
            self.walk.clone()
        };
        Some(image_cursor(handle, (32, 32)))
    }

    fn tile_highlight(&self, world: &mut World) -> Option<ActiveTileHighlight> {
        Some(ActiveTileHighlight {
            pos: self.target(world)?,
            image: self.highlight.clone(),
        })
    }
}
