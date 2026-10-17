use bevy::prelude::*;
use design_tokens::{TextRole, color};
use egui::load::SizedTexture;
use egui::{Color32, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::{StatusCategory, StatusInstance, Statuses};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::time::UnixMillis;
use crate::systems::player::session;

const ICON: f32 = 24.0;
const GAP: f32 = 4.0;

/// The statuses on the character as rows of icons hanging below the character panel, as wide as
/// the panel, a buff ringed green and a debuff red, each with a card of what it is and how long it
/// has left.
pub fn grid(ui: &mut Ui, world: &mut World) {
    let Some(me) = session::my_character(world) else {
        return;
    };
    let statuses = me.get::<Statuses>().cloned().unwrap_or_default();
    if statuses.0.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let now = session::server_now(world);
    ui.add_space(GAP);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(GAP);
        for status in &statuses.0 {
            let def = status.effect.get(&content);
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::hover());
            if let Some(image) = gui::image(world, &def.icon) {
                egui::Image::new(SizedTexture::new(image.texture, Vec2::splat(ICON)))
                    .paint_at(ui, rect);
            }
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, ring(def.category)),
                StrokeKind::Outside,
            );
            let label = format!("Status {}: {}", def.name, def.category.label());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
            });
            response.on_hover_ui(|ui| card(ui, def, status, now));
        }
    });
}

pub fn ring(category: StatusCategory) -> Color32 {
    match category {
        StatusCategory::Buff => color::OK,
        StatusCategory::Debuff => color::DANGER,
    }
}

pub fn time_left(status: &StatusInstance, now: Option<UnixMillis>) -> String {
    match (status.until, now) {
        (Some(until), Some(now)) if until > now => {
            let left = until.since(now).0;
            if left >= 60.0 {
                format!("{:.0} min left", (left / 60.0).ceil())
            } else {
                format!("{:.0} s left", left.ceil())
            }
        }
        (Some(_), _) => "Ending".to_owned(),
        (None, _) => "Until removed".to_owned(),
    }
}

fn card(
    ui: &mut Ui,
    def: &super::StatusEffectDef,
    status: &StatusInstance,
    now: Option<UnixMillis>,
) {
    ui.set_max_width(220.0);
    ui.label(RichText::new(&def.name).font(TextRole::Heading.font_id()));
    ui.label(
        RichText::new(def.category.label())
            .size(TextRole::Small.size())
            .color(ring(def.category)),
    );
    if !def.description.is_empty() {
        ui.label(RichText::new(&def.description).size(TextRole::Prose.size()));
    }
    ui.label(
        RichText::new(time_left(status, now))
            .size(TextRole::Small.size())
            .color(color::MUTED),
    );
}
