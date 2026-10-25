use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::render::world::decal::GroundDecal;
use crate::core::render::{self, Pointed};
use crate::core::units::Meters;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::input::map::{self, InputAction};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::player::session;
use crate::systems::{area, combat, movement};

pub struct WalkGesture;

/// The point last asked for. A held button asks again as soon as the pointer moves, so a walk led
/// by the pointer never reaches its end and waits for the next ask.
#[derive(Resource, Default)]
struct WalkState {
    last: Option<Pos<Meters>>,
}

impl Gesture for WalkGesture {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        let point = pointed?.ground?;
        area::walkable(world, point).then_some(ClickClaim::Walk(point))
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if !start {
            repeat(world);
            return;
        }
        if let Some(ClickClaim::Walk(point)) = claim {
            movement::request_walk(world, point);
            marks::walk_to(world, point);
            stamp(world, point);
        }
    }

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        let ClickClaim::Walk(point) = claim else {
            return None;
        };
        let shape = if area::warp_at(world, point).is_some() {
            CursorShape::Enter
        } else if map::pressed(world, InputAction::Interact) {
            CursorShape::WalkHeld
        } else {
            CursorShape::Walk
        };
        Some(Cursor::Shaped(shape))
    }

    fn mark(&self, _world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Walk(point) = claim else {
            return None;
        };
        Some(marks::hover_ring(point))
    }
}

fn repeat(world: &mut World) {
    let Some(pointed) = render::pointed(world) else {
        return;
    };
    let Some(point) = pointed.ground else {
        return;
    };
    if combat::enemy_at(world, &pointed).is_some() {
        return;
    }
    let last = world
        .get_resource::<WalkState>()
        .and_then(|state| state.last);
    if !area::walkable(world, point) || last == Some(point) {
        return;
    }
    movement::request_walk(world, point);
    stamp(world, point);
}

fn stamp(world: &mut World, point: Pos<Meters>) {
    world.get_resource_or_insert_with(WalkState::default).last = Some(point);
}
