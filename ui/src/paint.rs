//! Shapes painted straight onto a painter: dashes, hatching, washes and lines that measure.

use egui::emath::GuiRounding;
use egui::epaint::{Mesh, Vertex};
use egui::{
    Align2, Color32, Context, FontId, Painter, Pos2, Rect, Shape, Stroke, TextureId, Ui, Vec2,
};

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
/// Makes an ink print of a picture of four bytes a pixel, red, green, blue and how opaque: ink
/// where it was dark and bare paper where it was light, whatever its colours. Its tones are
/// stretched from its darkest to its lightest first, so a dim picture prints as firmly as a
/// bright one.
pub fn print(pixels: &mut [u8]) {
    let shade = |pixel: &[u8; 4]| {
        let lit = 0.2126 * f32::from(pixel[0])
            + 0.7152 * f32::from(pixel[1])
            + 0.0722 * f32::from(pixel[2]);
        let opaque = f32::from(pixel[3]) / 255.0;
        (255.0 - (255.0 - lit) * opaque).round() as u8
    };
    let pixels = pixels.as_chunks_mut::<4>().0;
    let mut counts = [0usize; 256];
    for pixel in pixels.iter() {
        counts[usize::from(shade(pixel))] += 1;
    }
    let reached = |share: f32| {
        let wanted = (pixels.len() as f32 * share) as usize;
        let mut passed = 0;
        counts
            .iter()
            .position(|count| {
                passed += count;
                passed > wanted
            })
            .unwrap_or(255) as f32
    };
    let (darkest, lightest) = (reached(PRINT_CLIPPED), reached(1.0 - PRINT_CLIPPED));
    let range = (lightest - darkest).max(1.0);
    let [red, green, blue, _] = ink::INK.to_array();
    for pixel in pixels {
        let dark = 1.0 - ((f32::from(shade(pixel)) - darkest) / range).clamp(0.0, 1.0);
        let inked = dark * dark * (3.0 - 2.0 * dark) * PRINT_DEPTH;
        *pixel = [red, green, blue, (inked * 255.0).round() as u8];
    }
}

const PRINT_CLIPPED: f32 = 0.02;
const PRINT_DEPTH: f32 = 0.9;

const FADE_CELLS: usize = 24;

/// The part `uv` of a picture drawn over `rect`, as opaque at each point as `opacity` says of
/// where the point lies in the rect, from its top left at (0, 0) to its bottom right at (1, 1):
/// a picture that fades into what lies under it.
pub fn faded_image(
    painter: &Painter,
    texture: TextureId,
    rect: Rect,
    uv: Rect,
    opacity: impl Fn(Vec2) -> f32,
) {
    let stride = FADE_CELLS + 1;
    let mut mesh = Mesh::with_texture(texture);
    for index in 0..stride * stride {
        let at = Vec2::new(
            (index % stride) as f32 / FADE_CELLS as f32,
            (index / stride) as f32 / FADE_CELLS as f32,
        );
        mesh.vertices.push(Vertex {
            pos: rect.min + rect.size() * at,
            uv: uv.min + uv.size() * at,
            color: Color32::WHITE.gamma_multiply(opacity(at).clamp(0.0, 1.0)),
        });
    }
    let stride = stride as u32;
    let cells = FADE_CELLS as u32;
    mesh.indices = (0..cells)
        .flat_map(|row| (0..cells).map(move |column| row * stride + column))
        .flat_map(|a| [a, a + 1, a + stride, a + 1, a + stride + 1, a + stride])
        .collect();
    painter.add(Shape::mesh(mesh));
}

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

pub fn ticked_bar(painter: &Painter, rect: Rect, share: f32, color: Color32, parts: u32) {
    painter.rect_filled(rect, 2.0, ink::ink_wash(0.2));
    let filled = Rect::from_min_max(
        rect.min,
        Pos2::new(
            rect.min.x + rect.width() * share.clamp(0.0, 1.0),
            rect.max.y,
        ),
    );
    painter.rect_filled(filled, 2.0, color);
    for part in 1..parts {
        let x = rect.min.x + rect.width() * part as f32 / parts as f32;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, ink::ink_wash(0.5)));
    }
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, ink::INK2),
        egui::StrokeKind::Inside,
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
