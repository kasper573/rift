use bevy::prelude::*;
use ui::egui;
use ui::egui::load::SizedTexture;
use ui::egui::{Align, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use ui::{ink, paint};

use super::{StatusCategory, StatusInstance, Statuses};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::time::{self, UnixMillis};
use crate::systems::hud;
use crate::systems::player::session;
use crate::systems::text::duration;

const ICON: f32 = 24.0;
const GAP: f32 = 5.0;
const RING: f32 = 2.0;

pub fn row(ui: &mut Ui, world: &mut World, width: f32, first_row: Align) {
    let Some(me) = session::my_character(world) else {
        return;
    };
    let statuses = me.get::<Statuses>().cloned().unwrap_or_default();
    if statuses.0.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let now = time::server_now(world);
    let pitch = ICON + GAP;
    let across = (((width + GAP) / pitch).floor() as usize).max(1);
    let columns = statuses.0.len().min(across);
    let rows = statuses.0.len().div_ceil(across);
    let size = Vec2::new(columns as f32, rows as f32) * pitch - Vec2::splat(GAP);
    let (taken, _) = ui.allocate_exact_size(size, Sense::hover());
    for (index, status) in statuses.0.iter().enumerate() {
        let def = status.effect.get(&content);
        let (column, row) = ((index % across) as f32, (index / across) as f32);
        let top = match first_row {
            Align::Max => taken.max.y - ICON - row * pitch,
            Align::Min | Align::Center => taken.min.y + row * pitch,
        };
        let rect = Rect::from_min_size(
            Pos2::new(taken.max.x - ICON - column * pitch, top),
            Vec2::splat(ICON),
        );
        let response = ui.interact(rect, ui.id().with(("status", index)), Sense::hover());
        ui.painter().rect_filled(rect.expand(RING), 3.0, ink::FRAME);
        if let Some(image) = gui::image(world, &def.icon) {
            egui::Image::new(SizedTexture::new(image.texture, Vec2::splat(ICON)))
                .paint_at(ui, rect);
        }
        ring(ui, rect.expand(RING), def.category);
        let label = format!("Status {}: {}", def.name, def.category.label());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
        });
        let lines: Vec<String> = [
            Some(format!(
                "{} · {}",
                def.category.label(),
                time_left(status, now)
            )),
            (!def.description.is_empty()).then(|| def.description.clone()),
        ]
        .into_iter()
        .flatten()
        .collect();
        hud::tooltip(&response, world, &def.name, &lines, None);
    }
}

pub fn time_left(status: &StatusInstance, now: Option<UnixMillis>) -> String {
    match (status.until, now) {
        (Some(until), Some(now)) if until > now => format!("{} left", duration(until.since(now))),
        (Some(_), _) => "Ending".to_owned(),
        (None, _) => "Until removed".to_owned(),
    }
}

// The ring's pattern says the category as well as its colour does.
fn ring(ui: &Ui, rect: Rect, category: StatusCategory) {
    match category {
        StatusCategory::Buff => {
            ui.painter()
                .rect_stroke(rect, 3.0, Stroke::new(RING, BUFF), StrokeKind::Inside);
        }
        StatusCategory::Debuff => {
            paint::dashed_rect(ui.painter(), rect, Stroke::new(RING, DEBUFF), 5.0, 3.0);
        }
    }
}

const BUFF: Color32 = Color32::from_rgb(0x6F, 0xC2, 0x86);
const DEBUFF: Color32 = Color32::from_rgb(0xE0, 0x6A, 0x5C);
