//! The grid of item slots. Every window that lays slots out in rows (the bag, what a trade, a
//! letter, a listing, a sale or a prompt holds) draws them through [`item_grid`], so grids side by
//! side line up slot for slot, and sizes its room from the measures here.

use ui::egui::{self, Response, Ui, Vec2};
use ui::sheet;

use super::widget::SLOT;

/// The room between two slots, across and down.
pub const SLOT_GAP: f32 = 4.0;

/// How long a run of `slots` slots is, across or down.
pub const fn slots_span(slots: u32) -> f32 {
    slots as f32 * SLOT + slots.saturating_sub(1) as f32 * SLOT_GAP
}

/// How many slots stand side by side in a grid `width` wide.
pub fn columns_in(width: f32) -> u32 {
    (((width + SLOT_GAP) / (SLOT + SLOT_GAP)).floor() as u32).max(1)
}

/// How tall a grid of `cells` slots stands `width` wide: at least a row.
pub fn grid_height(cells: u32, width: f32) -> f32 {
    slots_span(cells.div_ceil(columns_in(width)).max(1))
}

/// `cells` slots in rows as wide as the room, each drawn by `cell` from its place in the grid.
pub fn item_grid(ui: &mut Ui, cells: u32, mut cell: impl FnMut(&mut Ui, u32)) -> Response {
    let across = columns_in(ui.available_width());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
        ui.vertical(|ui| {
            for first in (0..cells).step_by(across as usize) {
                ui.horizontal(|ui| {
                    for place in first..(first + across).min(cells) {
                        cell(ui, place);
                    }
                });
            }
        });
    })
    .response
}

/// A pane of slots that scrolls inside the room left to it, so a grid with more rows than fit
/// never runs past what holds the pane.
pub fn scrolled<R>(ui: &mut Ui, id_salt: impl egui::AsIdSalt, add: impl FnOnce(&mut Ui) -> R) -> R {
    let room = ui.available_rect_before_wrap();
    sheet::scroll_pane(
        ui,
        sheet::Pane::bare(room),
        egui::ScrollArea::vertical().id_salt(id_salt),
        add,
    )
}
