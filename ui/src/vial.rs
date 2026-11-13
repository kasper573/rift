use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::face::Face;
use crate::glyph::{self, Halo};
use crate::{ink, slip};

pub struct Vial<'a> {
    pub full: f32,
    pub amount: &'a str,
    pub of: &'a str,
    pub low: bool,
}

pub const SIZE: f32 = 72.0;

pub fn paint(painter: &Painter, center: Pos2, vial: &Vial, seconds: f64) {
    let radius = SIZE / 2.0;
    let around = Rect::from_center_size(center, Vec2::splat(SIZE));
    painter.add(slip::SHADOW.as_shape(around, CornerRadius::same(radius as u8)));
    painter.circle_filled(center, radius, ink::FRAME);
    let beat = if vial.low {
        0.5 + 0.5 * (seconds * std::f64::consts::TAU / PULSE_SECONDS).sin() as f32
    } else {
        0.0
    };
    painter.circle_stroke(
        center,
        radius - 2.0,
        Stroke::new(2.0 + beat, ink::on(ink::GOLD, LOW_RIM, beat)),
    );
    let glass = radius - RIM;
    painter.circle_filled(center, glass, GLASS);
    liquid(painter, center, glass, vial.full.clamp(0.0, 1.0));
    painter.circle_filled(
        center + Vec2::new(-0.4, -0.48) * glass,
        glass * 0.2,
        Color32::from_white_alpha(26),
    );
    let lettering = if vial.low {
        ink::on(ink::LIGHT, GLASS, 0.35)
    } else {
        ink::LIGHT
    };
    glyph::haloed(
        painter,
        center + Vec2::new(0.0, 2.0),
        Align2::CENTER_BOTTOM,
        vial.amount,
        Face::BookBold.id(21.0),
        lettering,
        HALO,
    );
    glyph::haloed(
        painter,
        center + Vec2::new(0.0, 1.0),
        Align2::CENTER_TOP,
        vial.of,
        Face::Book.id(11.5),
        lettering,
        HALO,
    );
}

const RIM: f32 = 5.0;
const PULSE_SECONDS: f64 = 1.1;
const GLASS: Color32 = Color32::from_rgb(0x1C, 0x13, 0x10);
const LOW_RIM: Color32 = Color32::from_rgb(0xE8, 0x4A, 0x3A);
const SURFACE: Color32 = Color32::from_rgb(0xD0, 0x5A, 0x4C);
const HALO: Halo = Halo {
    color: GLASS,
    width: 1.0,
};
const ARC_STEPS: usize = 40;

// egui clips to rects alone, so the round edge is the shape's own.
fn liquid(painter: &Painter, center: Pos2, glass: f32, full: f32) {
    if full <= 0.0 {
        return;
    }
    if full >= 1.0 {
        painter.circle_filled(center, glass, ink::WAX);
        return;
    }
    let level = glass - 2.0 * glass * full;
    let from = (level / glass).asin();
    let to = std::f32::consts::PI - from;
    let rim: Vec<Pos2> = (0..=ARC_STEPS)
        .map(|step| {
            let angle = from + (to - from) * step as f32 / ARC_STEPS as f32;
            center + Vec2::new(angle.cos(), angle.sin()) * glass
        })
        .collect();
    let surface = [rim[rim.len() - 1], rim[0]];
    painter.add(Shape::convex_polygon(rim, ink::WAX, Stroke::NONE));
    painter.line_segment(surface, Stroke::new(1.5, SURFACE));
}
