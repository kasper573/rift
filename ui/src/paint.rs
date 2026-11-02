//! Shapes painted straight onto a painter: dashes, hatching, washes and lines that measure.

use egui::emath::GuiRounding;
use egui::epaint::Mesh;
use egui::{Align2, Color32, Context, FontId, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2};

use crate::{glyph, ink, theme};

/// A copy of text drawn under it, `offset` points away, in `color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset: Vec2,
    pub color: Color32,
}

/// The edge of `rect` in dashes of `dash` with `gap` between, laid inside it on whole pixels.
pub fn dashed_rect(painter: &Painter, rect: Rect, stroke: Stroke, dash: f32, gap: f32) {
    let rect = rect.round_to_pixels(painter.pixels_per_point());
    let mut mesh = Mesh::default();
    let mut along = |from: Pos2, direction: Vec2, length: f32, thickness: Vec2| {
        let mut at = 0.0;
        while at < length {
            let piece = dash.min(length - at);
            let start = from + direction * at;
            mesh.add_colored_rect(
                Rect::from_two_pos(start, start + direction * piece + thickness),
                stroke.color,
            );
            at += dash + gap;
        }
    };
    let (width, height, thick) = (rect.width(), rect.height(), stroke.width);
    along(rect.left_top(), Vec2::X, width, Vec2::Y * thick);
    along(
        rect.left_bottom() - Vec2::Y * thick,
        Vec2::X,
        width,
        Vec2::Y * thick,
    );
    along(rect.left_top(), Vec2::Y, height, Vec2::X * thick);
    along(
        rect.right_top() - Vec2::X * thick,
        Vec2::Y,
        height,
        Vec2::X * thick,
    );
    painter.add(Shape::mesh(mesh));
}

/// `rect` washed in `color` and hatched over in it, the lines further apart the larger `scale`.
pub fn hatched(painter: &Painter, rect: Rect, color: Color32, scale: f32) {
    painter.rect_filled(
        rect,
        0.0,
        ink::wash(color, f32::from(color.a()) / 255.0 / 3.0),
    );
    glyph::hatch_every(painter, rect, color, (scale / 2.0).clamp(3.0, 12.0));
}

/// `rect` washed in `color` and ruled over in it across.
pub fn striped(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, ink::wash(color, 0.2));
    let step = (rect.height() / 4.0).max(3.0);
    let stroke = Stroke::new(1.0, color);
    let mut y = rect.min.y + step / 2.0;
    while y < rect.max.y {
        painter.hline(rect.x_range(), y, stroke);
        y += step;
    }
}

/// Dots every `step` across `rect`, one on `origin`, so a canvas's dots move with what is on it.
pub fn dot_grid(painter: &Painter, rect: Rect, origin: Pos2, step: f32, color: Color32) {
    let first = |origin: f32, min: f32| origin + ((min - origin) / step).floor() * step;
    let mut y = first(origin.y, rect.min.y);
    while y <= rect.max.y {
        let mut x = first(origin.x, rect.min.x);
        while x <= rect.max.x {
            painter.circle_filled(Pos2::new(x, y), 1.0, color);
            x += step;
        }
        y += step;
    }
}

/// The wash under a row the pointer is on or that is picked.
pub fn row_wash(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 3.0, theme::ACCENT_SOFT);
}

/// A thin track the width of `size` filled to `share` of it in `color`, as long as something has
/// left.
pub fn progress_line(ui: &mut Ui, size: Vec2, share: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let y = rect.center().y;
    let painter = ui.painter();
    painter.hline(rect.x_range(), y, Stroke::new(1.0, ink::ink_wash(0.4)));
    painter.hline(
        rect.min.x..=rect.min.x + rect.width() * share.clamp(0.0, 1.0),
        y,
        Stroke::new(size.y, color),
    );
}

/// The size `text` lays out to on one line in `font`.
pub fn text_size(gui: &Context, text: &str, font: FontId) -> Vec2 {
    gui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
            .size()
    })
}

/// `text` on one line `anchor`ed at `at`, cut short with an ellipsis past `width`.
pub fn text_within(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    ink: Color32,
    width: f32,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, ink);
    rect
}
