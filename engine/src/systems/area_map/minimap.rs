use bevy::prelude::*;
use ui::egui::{self, CornerRadius, Pos2, Sense, Stroke, Ui, Vec2};
use ui::{ink, slip};

use super::{MapView, MapWindow};
use crate::core::content::Content;
use crate::core::units::Radians;
use crate::systems::hud::menu::{self, MenuTabName};
use crate::systems::hud::{self, HudWidget, day_clock};
use crate::systems::settings::UserSettings;

pub const MINIMAP: &str = "minimap";

/// The room the minimap rests in: its disc, what goes round it, and the area's name under it.
pub fn resting_size(world: &World) -> Vec2 {
    let round = DISC + 2.0 * overhang(world);
    Vec2::new(round, round + NAME_ROOM)
}

pub struct MinimapWidget;

impl HudWidget for MinimapWidget {
    fn fallback(&self, world: &World, screen: egui::Rect) -> Pos2 {
        Pos2::new(
            screen.right() - resting_size(world).x - hud::MARGIN,
            screen.top() + hud::MARGIN,
        )
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<egui::Rect> {
        let overhang = overhang(world);
        let (piece, _) = ui.allocate_exact_size(resting_size(world), Sense::hover());
        let center = piece.center_top() + Vec2::new(0.0, overhang + DISC / 2.0);
        let whole = egui::Rect::from_center_size(center, Vec2::splat(DISC));
        ui.painter()
            .add(slip::SHADOW.as_shape(whole, CornerRadius::same((DISC / 2.0) as u8)));
        if day_clock::shown(world) {
            day_clock::face(ui, world, center, DISC / 2.0);
        }
        let user = world.resource::<UserSettings>();
        let (chosen, turning) = (user.minimap_reach(), user.minimap_turns());
        let view = MapView::of(
            world,
            |reaches, sight| chosen.unwrap_or(reaches.step_reaching(sight)),
            |_| MapWindow::Disc { center, turning },
        );
        let Some(view) = view else {
            ui.painter().circle_filled(center, GROUND, super::BLANK);
            rim(ui, center);
            return None;
        };
        let content = world.resource::<Content>().clone();
        let area_name = view.area.get(&content).name.clone();
        let disc = ui
            .interact(whole, ui.id().with("disc"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        hud::grab_spot(world, MINIMAP, &disc, piece);
        let reach = view.reach.0;
        disc.widget_info(|| {
            let said = format!("Minimap of {area_name}, reaching {reach} m");
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, said)
        });
        super::chart(ui, world, &view);
        rim(ui, center);
        super::north_bead(ui, world, center + view.north() * (GROUND + RIM / 2.0));
        let buttons =
            [CLOSER_AT, WIDER_AT].map(|angle| center + Vec2::angled(angle.0) * (DISC / 2.0 - 1.0));
        let stepped = super::reach_buttons(ui, &view, buttons)
            .or_else(|| super::wheeled(ui, world, &disc, &view));
        if let Some(step) = stepped {
            world.resource_mut::<UserSettings>().set_minimap_reach(step);
        }
        super::lettered(
            ui,
            &area_name,
            center + Vec2::new(0.0, DISC / 2.0 + overhang),
        );
        if disc.clicked() {
            menu::toggle(world, MenuTabName::Map);
        }
        None
    }
}

pub(super) const GROUND: f32 = DISC / 2.0 - RIM;

const DISC: f32 = 164.0;
const RIM: f32 = 6.0;
const RIM_OVERHANG: f32 = 7.0;
const NAME_ROOM: f32 = 22.0;
// Clockwise from the rim's right end.
const CLOSER_AT: Radians = Radians(0.56);
const WIDER_AT: Radians = Radians(0.84);

/// How far past the disc what goes round it reaches: its rim, or the clock when that is shown.
fn overhang(world: &World) -> f32 {
    if day_clock::shown(world) {
        day_clock::REACH
    } else {
        RIM_OVERHANG
    }
}

fn rim(ui: &Ui, center: Pos2) {
    let painter = ui.painter();
    painter.circle_stroke(center, GROUND + RIM / 2.0, Stroke::new(RIM, ink::FRAME));
    painter.circle_stroke(center, GROUND, Stroke::new(1.0, ink::LIGHT));
    painter.circle_stroke(center, DISC / 2.0, Stroke::new(1.5, ink::GOLD));
}
