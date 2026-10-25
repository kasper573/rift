use engine::core::math::{Offset, Pos, Rect as TileRect};
use ui::egui;
use ui::egui::{Pos2, Rect, Response, Ui, Vec2};

use crate::keymap::EditorKey;

#[derive(Clone, Copy)]
pub struct Viewport<U> {
    pub screen: Rect,
    pub center: Pos<U>,
    pub zoom: f32,
}

/// How many canvas pixels a metre of the world takes at a zoom of one.
pub const PX_PER_METER: f32 = 16.0;

impl<U> Viewport<U> {
    pub fn scale(&self) -> f32 {
        self.zoom * PX_PER_METER
    }

    pub fn to_screen(&self, p: Pos<U>) -> Pos2 {
        let c = self.screen.center();
        let d = (p - self.center) * self.scale();
        Pos2::new(c.x + d.x, c.y + d.y)
    }

    pub fn to_tile(&self, p: Pos2) -> Pos<U> {
        self.center + offset(p - self.screen.center()) / self.scale()
    }

    pub fn rect(&self, r: TileRect<U>) -> Rect {
        Rect::from_min_max(self.to_screen(r.min()), self.to_screen(r.max()))
    }

    pub fn visible(&self) -> TileRect<U> {
        TileRect::from_points([self.to_tile(self.screen.min), self.to_tile(self.screen.max)])
    }
}

pub fn zoom_and_pan<U>(
    ui: &Ui,
    response: &Response,
    viewport: &mut Viewport<U>,
    zoom_range: (f32, f32),
) -> bool {
    let scroll = ui.input(|i| i.smooth_scroll_delta);
    let zoom_delta = ui.input(|i| i.zoom_delta());
    if response.hovered() && (scroll.y != 0.0 || zoom_delta != 1.0) {
        let factor = if zoom_delta != 1.0 {
            zoom_delta
        } else {
            (scroll.y / 240.0).exp2()
        };
        let before = viewport.zoom;
        let under = response
            .hover_pos()
            .map(|pointer| viewport.to_tile(pointer));
        viewport.zoom = (before * factor).clamp(zoom_range.0, zoom_range.1);
        if let Some(under) = under {
            viewport.center = under + (viewport.center - under) * (before / viewport.zoom);
        }
    }
    let space = ui.input(|i| EditorKey::Pan.held(i));
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || (space && response.dragged_by(egui::PointerButton::Primary));
    if panning {
        viewport.center -= offset(response.drag_delta()) / viewport.scale();
    }
    panning
}

pub fn pixels_per_unit(available: Vec2, content: Vec2) -> f32 {
    (available.x / content.x.max(1.0)).min(available.y / content.y.max(1.0))
}

pub fn fit_zoom(available: Vec2, content_tiles: Vec2, tile_px: f32) -> f32 {
    pixels_per_unit(available, content_tiles) / tile_px
}

fn offset<U>(v: Vec2) -> Offset<U> {
    Offset::new(v.x, v.y)
}

pub fn pos_from_egui<U>(p: Pos2) -> Pos<U> {
    Pos::new(p.x, p.y)
}

pub fn rect_from_egui<U>(r: Rect) -> TileRect<U> {
    TileRect::from_points([pos_from_egui(r.min), pos_from_egui(r.max)])
}

pub fn rect_to_egui<U>(r: TileRect<U>) -> Rect {
    Rect::from_min_max(
        Pos2::new(r.min_x(), r.min_y()),
        Pos2::new(r.max_x(), r.max_y()),
    )
}
