//! Which way is north and how close the camera looks, since the camera turns and zooms: a needle
//! that points north on the screen, and the zoom step under it.

use bevy::prelude::*;
use ui::egui::{self, Align2, Color32, Pos2, Rect, Stroke, Ui};
use ui::face::TextRole;
use ui::ink;

use super::{HudWidget, MARGIN};
use crate::core::render::{self, CameraRig, Zoom};

const SIZE: f32 = 44.0;
const NEEDLE: f32 = 10.0;
const LABEL: f32 = 16.0;
const DOT: f32 = 3.0;
const DOT_GAP: f32 = 8.0;
const RIM: Color32 = ink::INK3;
const NORTH: Color32 = ink::GOLD2;

pub struct CompassWidget;

impl HudWidget for CompassWidget {
    fn fallback(&self, screen: Rect) -> Pos2 {
        Pos2::new(
            screen.left() + MARGIN,
            screen.bottom() - MARGIN - SIZE - DOT * 2.0 - DOT_GAP,
        )
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(north) = north_on_screen(world) else {
            return;
        };
        let zoom = world.resource::<CameraRig>().zoom().step();
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(SIZE, SIZE + DOT * 2.0 + DOT_GAP),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        let center = Pos2::new(rect.center().x, rect.top() + SIZE / 2.0);
        painter.circle(
            center,
            SIZE / 2.0 - 1.0,
            ink::wash(ink::VEIL, 0.5),
            Stroke::new(1.0, RIM),
        );
        let tip = center + north * NEEDLE;
        let tail = center - north * NEEDLE * 0.5;
        painter.line_segment([center, tail], Stroke::new(2.0, RIM));
        painter.line_segment([center, tip], Stroke::new(2.0, NORTH));
        painter.text(
            center + north * LABEL,
            Align2::CENTER_CENTER,
            "N",
            TextRole::Name.font_id(),
            NORTH,
        );
        let steps = Zoom::STEPS;
        let row = rect.bottom() - DOT;
        let left = rect.center().x - (steps - 1) as f32 * DOT_GAP / 2.0;
        for step in 0..steps {
            let at = Pos2::new(left + step as f32 * DOT_GAP, row);
            let lit = step == zoom;
            painter.circle_filled(at, DOT, if lit { NORTH } else { RIM });
        }
    }
}

/// North as a unit direction on the screen, y down.
fn north_on_screen(world: &mut World) -> Option<egui::Vec2> {
    let view = render::view_of(world)?;
    let target = world.resource::<CameraRig>().target();
    let here = view.project(target)?;
    let north = view.project(target + Vec3::NEG_Z)?;
    let toward = north - here;
    (toward.length() > 1e-3).then(|| {
        let unit = toward.normalize();
        egui::vec2(unit.x, unit.y)
    })
}
