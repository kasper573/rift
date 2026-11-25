//! The windows a conversation opens: each is the player's to move and size, and remembers where
//! they left it. Until they place one it stands right above the dialogue, side by side with the
//! other windows the conversation has up.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ui::egui::{self, Pos2, Rect, Ui, Vec2};

use super::stage;
use crate::systems::hud::{self, Seat, Sizing, WindowKey};
use crate::systems::settings::{Placed, UserSettings};

const GAP: f32 = 12.0;
const SCREEN_SHARE: f32 = 0.94;
const SCREEN_MARGIN: f32 = 8.0;
const BELOW_LIFT: f32 = 14.0;

/// A window of a conversation: its `size` as it starts, and the least the player may size it to.
#[derive(Clone, Copy)]
pub(crate) struct Opens {
    pub key: WindowKey,
    pub of: Placed,
    pub size: Vec2,
    pub min: Vec2,
}

/// Draws the window of `opens` while `open` holds a view, and ends the conversation when the player
/// closes it, as it is part of the conversation.
pub(crate) fn window<V, R>(
    world: &mut World,
    gui: &egui::Context,
    opens: Opens,
    open: Option<V>,
    title: impl FnOnce(&V) -> String,
    add: impl FnOnce(&mut Ui, &mut World, &V) -> R,
) -> Option<R> {
    let placed = world
        .resource::<UserSettings>()
        .placement(opens.of)
        .is_some();
    let start = seat(world, gui, opens.key, opens.size, !placed && open.is_some());
    let seat = Seat {
        key: opens.key,
        of: opens.of,
        start,
        sizing: Sizing::Resizable { min: opens.min },
    };
    let drawn = hud::viewed(world, gui, seat, open, title, add)?;
    if drawn.closed {
        stage::leave(world);
    }
    Some(drawn.inner)
}

/// Where the window of `key` and `size` starts: in the row of every window the conversation has
/// up that the player hasn't placed, centred above the dialogue. A window that is not `in_row`
/// has been placed by the player, and keeps out of it.
pub(crate) fn seat(
    world: &mut World,
    gui: &egui::Context,
    key: WindowKey,
    size: Vec2,
    in_row: bool,
) -> Rect {
    let screen = gui.content_rect();
    let size = Vec2::new(size.x.min(screen.width() * SCREEN_SHARE), size.y);
    let bottom = stage::dialogue_rect(world).map_or(screen.bottom() - BELOW_LIFT, |dialogue| {
        dialogue.top() - GAP
    });
    let height = size.y.min((bottom - screen.top() - SCREEN_MARGIN).max(0.0));
    let mut row = world.resource::<Seating>().last.clone();
    if in_row {
        world.resource_mut::<Seating>().now.insert(key, size);
        row.insert(key, size);
    }
    let total: f32 =
        row.values().map(|size| size.x).sum::<f32>() + GAP * row.len().saturating_sub(1) as f32;
    let before: f32 = row.range(..key).map(|(_, size)| size.x + GAP).sum();
    let left = screen.center().x - total / 2.0 + before;
    Rect::from_min_size(Pos2::new(left, bottom - height), Vec2::new(size.x, height))
}

/// Where the window of `key` and `size` starts when it should keep the people talking in sight:
/// above the dialogue, where it covers the least of them as the camera frames them once it comes
/// to rest, and of those spots the nearest its seat in the row. The camera may still be on its
/// way, so the spot is worth working out once and keeping, never following the camera.
pub(crate) fn seat_in_sight_of_bodies(
    world: &mut World,
    gui: &egui::Context,
    key: WindowKey,
    size: Vec2,
) -> Rect {
    let row = seat(world, gui, key, size, true);
    let screen = gui.content_rect().shrink(SCREEN_MARGIN);
    let within = Rect::from_min_max(screen.min, Pos2::new(screen.max.x, row.bottom()));
    let drawn = world.resource::<crate::core::gui::WidgetRects>();
    let others: Vec<Rect> = WindowKey::OF_A_CONVERSATION
        .iter()
        .filter(|other| **other != key)
        .filter_map(|other| drawn.0.get(other.name()).copied())
        .collect();
    let kept_clear: Vec<Rect> = super::camera::bodies_at_rest(world)
        .into_iter()
        .chain(others)
        .collect();
    let at = ui::float::clear_spot(row.size(), within, &kept_clear, row.min, GAP);
    Rect::from_min_size(at, row.size())
}

/// What the windows drawn this pass stand to start at, which the next pass lays its row out by.
#[derive(Resource, Default)]
pub(super) struct Seating {
    last: BTreeMap<WindowKey, Vec2>,
    now: BTreeMap<WindowKey, Vec2>,
}

pub(super) fn begin_pass(world: &mut World) {
    let mut seating = world.resource_mut::<Seating>();
    seating.last = std::mem::take(&mut seating.now);
}
