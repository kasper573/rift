use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::item::{DroppedItem, Reservation, ReservedBy};
use crate::systems::movement::Position;
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::core::render;
use crate::systems::input::gestures::{Gesture, image_cursor};

pub struct PickupGesture;

impl Gesture for PickupGesture {
    fn priority(&self) -> i32 {
        2
    }

    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world)
            && render::cursor_tile(world)
                .and_then(|point| item_at(world, point))
                .is_some()
    }

    fn drive(&self, world: &mut World, start: bool) {
        if start
            && let Some(point) = render::cursor_tile(world)
            && let Some(target) = item_at(world, point)
        {
            session::pickup(world, target);
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let handle = world
            .resource::<AssetServer>()
            .load("icons/cursors/hand001.png");
        Some(image_cursor(handle, (8, 8)))
    }
}

fn item_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::my_id(world);
    let cell = point.cell();
    let mut items = world.query::<(Entity, &Position, &DroppedItem, Option<&Reservation>)>();
    items.iter(world).find_map(|(entity, at, _, reservation)| {
        let reserved = reservation.map_or(ReservedBy::None, |reservation| reservation.by);
        (at.pos.cell() == cell && reserved.allows(me)).then_some(entity)
    })
}
