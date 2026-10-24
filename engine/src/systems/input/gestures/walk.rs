use bevy::prelude::*;

use crate::core::content::{Content, Fixture};
use crate::core::math::Pos;
use crate::core::render;
use crate::core::units::{Meters, TilePos};
use crate::systems::input::gestures::{ActiveTileHighlight, ClickClaim, Cursor, Gesture};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::{CursorShape, InterfaceIcon};
use crate::systems::player::session;
use crate::systems::{area, combat, movement};

pub struct WalkGesture;

/// The tile last asked for. A held button asks again as soon as the pointer is over another tile,
/// so a walk led by the pointer never reaches its end and waits for the next ask.
#[derive(Resource, Default)]
struct WalkState {
    last_tile: Option<Pos<Meters>>,
}

impl Gesture for WalkGesture {
    fn claim(&self, world: &mut World, tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        let tile = tile?.snap();
        area::walkable(world, tile).then_some(ClickClaim::Walk(tile))
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if !start {
            repeat(world);
            return;
        }
        if let Some(ClickClaim::Walk(tile)) = claim {
            movement::request_walk(world, tile);
            stamp(world, tile);
        }
    }

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        let ClickClaim::Walk(_) = claim else {
            return None;
        };
        let shape = if map::pressed(world, InputAction::Interact) {
            CursorShape::WalkHeld
        } else {
            CursorShape::Walk
        };
        Some(Cursor::Shaped(shape))
    }

    fn tile_highlight(&self, world: &mut World, claim: ClickClaim) -> Option<ActiveTileHighlight> {
        let ClickClaim::Walk(pos) = claim else {
            return None;
        };
        let target = &InterfaceIcon::WalkTarget
            .get(world.resource::<Content>())
            .image;
        let image = world.resource::<AssetServer>().load(target.path());
        Some(ActiveTileHighlight { pos, image })
    }
}

fn repeat(world: &mut World) {
    let Some(point) = render::cursor_tile(world) else {
        return;
    };
    if combat::enemy_at(world, point).is_some() {
        return;
    }
    let tile = point.snap();
    let last_tile = world
        .get_resource::<WalkState>()
        .and_then(|state| state.last_tile);
    if !area::walkable(world, tile) || last_tile == Some(tile) {
        return;
    }
    movement::request_walk(world, tile);
    stamp(world, tile);
}

fn stamp(world: &mut World, tile: Pos<Meters>) {
    world
        .get_resource_or_insert_with(WalkState::default)
        .last_tile = Some(tile);
}
