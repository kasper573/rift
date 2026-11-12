//! A wheel of things to pick, laid round a point: what a held key opens round a character.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::{Align2, Image, Pos2, Rect, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};

use crate::face::TextRole;
use crate::{glyph, ink};

/// How wide the picture of a pick is drawn.
pub const PICTURE: f32 = 32.0;

const SLOT: f32 = 52.0;
/// How far from the wheel's middle its slots lie at the least, and how much room each takes of
/// the ring where more of them need a wider one.
const RADIUS: f32 = 96.0;
const SPACING: f32 = 1.5;

/// One thing to pick off a wheel: its name, and its picture once there is one to draw.
pub struct WheelPick<'a> {
    pub name: &'a str,
    pub image: Option<Image<'a>>,
}

/// Lays `picks` evenly round `center`, the first at the top and the rest clockwise, each a
/// round slot with its picture over its name. Returns the place of the one clicked.
pub fn wheel(ui: &mut Ui, center: Pos2, picks: Vec<WheelPick>) -> Option<usize> {
    let count = picks.len() as f32;
    let radius = RADIUS.max(SLOT * SPACING * count / TAU);
    let mut clicked = None;
    for (index, pick) in picks.into_iter().enumerate() {
        let angle = TAU * index as f32 / count - FRAC_PI_2;
        let middle = center + radius * Vec2::angled(angle);
        let rect = Rect::from_center_size(middle, Vec2::splat(SLOT));
        let response = ui.allocate_rect(rect, Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, pick.name));
        let (fill, rim) = if response.hovered() {
            (ink::HONEY, Stroke::new(2.0, ink::GOLD))
        } else {
            (ink::CREAM, Stroke::new(1.0, ink::INK2))
        };
        ui.painter().circle(middle, SLOT / 2.0, fill, rim);
        if let Some(image) = pick.image {
            image.paint_at(ui, Rect::from_center_size(middle, Vec2::splat(PICTURE)));
        }
        glyph::haloed(
            ui.painter(),
            rect.center_bottom() + Vec2::new(0.0, 3.0),
            Align2::CENTER_TOP,
            pick.name,
            TextRole::Small.font_id(),
            ink::INK,
            glyph::Halo {
                color: ink::CREAM,
                width: 1.0,
            },
        );
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}
