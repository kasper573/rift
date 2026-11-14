//! The seat a globe of liquid sits in on the interface: the dark socket behind its glass, the
//! rim round it, the amount written over it, and the liquid drawn flat for when nothing lights it.

use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::face::Face;
use crate::glyph::{self, Halo};
use crate::{ink, slip};

/// How wide a globe is in its seat, rim and all.
pub const SIZE: f32 = 72.0;
/// How far from its middle a globe's glass reaches.
pub const GLASS: f32 = SIZE / 2.0 - RIM;

/// The socket a globe's glass stands in, under the globe itself.
pub fn socket(painter: &Painter, center: Pos2) {
    let radius = SIZE / 2.0;
    let around = Rect::from_center_size(center, Vec2::splat(SIZE));
    painter.add(slip::SHADOW.as_shape(around, CornerRadius::same(radius as u8)));
    painter.circle_filled(center, radius, ink::FRAME);
    painter.circle_filled(center, GLASS, HOLLOW);
}

/// The rim round a globe, over the globe itself. One that warns beats in the warning's colour.
pub fn rim(painter: &Painter, center: Pos2, warns: bool, seconds: f64) {
    let beat = if warns {
        0.5 + 0.5 * (seconds * std::f64::consts::TAU / PULSE_SECONDS).sin() as f32
    } else {
        0.0
    };
    painter.circle_stroke(
        center,
        SIZE / 2.0 - 2.0,
        Stroke::new(2.0 + beat, ink::on(ink::GOLD, WARNING, beat)),
    );
}

/// How much a globe holds, written across its middle.
pub fn amount(painter: &Painter, center: Pos2, amount: &str) {
    glyph::haloed(
        painter,
        center,
        Align2::CENTER_CENTER,
        amount,
        Face::BookBold.id(AMOUNT),
        ink::LIGHT,
        HALO,
    );
}

/// The liquid of a globe filled to `full`, flat: what stands in for the lit globe until the
/// renderer has drawn it.
// egui clips to rects alone, so the round edge is the shape's own.
pub fn flat(painter: &Painter, center: Pos2, full: f32, liquid: Color32) {
    let full = full.clamp(0.0, 1.0);
    if full <= 0.0 {
        return;
    }
    if full >= 1.0 {
        painter.circle_filled(center, GLASS, liquid);
        return;
    }
    let level = GLASS - 2.0 * GLASS * full;
    let from = (level / GLASS).asin();
    let to = std::f32::consts::PI - from;
    let rim: Vec<Pos2> = (0..=ARC_STEPS)
        .map(|step| {
            let angle = from + (to - from) * step as f32 / ARC_STEPS as f32;
            center + Vec2::new(angle.cos(), angle.sin()) * GLASS
        })
        .collect();
    painter.add(Shape::convex_polygon(rim, liquid, Stroke::NONE));
}

const RIM: f32 = 5.0;
const AMOUNT: f32 = 15.0;
const PULSE_SECONDS: f64 = 1.1;
const HOLLOW: Color32 = Color32::from_rgb(0x1C, 0x13, 0x10);
const WARNING: Color32 = Color32::from_rgb(0xE8, 0x4A, 0x3A);
const HALO: Halo = Halo {
    color: HOLLOW,
    width: 1.5,
};
const ARC_STEPS: usize = 40;
