//! The rows of a form: a key beside its value, a section's heading, a rule, a hint, an error.

use std::fmt::Display;

use egui::{Color32, RichText, Ui, Vec2};

use crate::face;
use crate::face::{Face, TextRole};
use crate::{ink, sheet, theme};

const KEY_WIDTH: f32 = 96.0;

/// A row of `value` after its `key`, the keys of a form lined up in one column.
pub fn kv(ui: &mut Ui, key: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(KEY_WIDTH, ui.spacing().interact_size.y),
            egui::Label::new(theme::muted(key)),
        );
        value(ui);
    });
}

pub fn kv_text(ui: &mut Ui, key: &str, value: impl Into<String>) {
    kv(ui, key, |ui| {
        ui.add(egui::Label::new(value.into()).truncate());
    });
}

pub fn kv_code(ui: &mut Ui, key: &str, value: impl Into<String>) {
    kv(ui, key, |ui| {
        ui.add(egui::Label::new(face::mono(value.into())).truncate());
    });
}

pub fn section_heading(text: &str) -> RichText {
    RichText::new(text)
        .font(Face::SmallCapsBold.id(TextRole::Section.size()))
        .color(ink::RUBRIC)
}

pub fn section_count(ui: &mut Ui, text: &str, count: impl Display) {
    sheet::section(ui, &format!("{text} · {count}"));
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(theme::muted(text).size(12.5));
}

pub fn error_line(ui: &mut Ui, error: Option<&str>) {
    if let Some(error) = error {
        ui.colored_label(theme::DANGER, error);
    }
}

/// What a panel says while nothing in it is picked: how to pick something.
pub fn nothing_selected(ui: &mut Ui, call_to_action: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(theme::muted(call_to_action).size(13.0));
    });
}

/// A small caption, such as the time or the speaker of a line.
pub fn caption(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.label(TextRole::Small.rich(text.into()).color(color));
}

/// A hairline across the room at the cursor.
pub fn rule(ui: &Ui) {
    rule_with(ui, egui::Stroke::new(1.0, theme::BORDER));
}

/// A rule across the room at the cursor in `stroke`, for one on another ground or fainter.
pub fn rule_with(ui: &Ui, stroke: egui::Stroke) {
    ui.painter()
        .hline(ui.max_rect().x_range(), ui.cursor().min.y, stroke);
}

/// A caption centred between two hairlines, parting what comes before from what comes after.
pub fn divider(ui: &mut Ui, text: impl Into<String>) {
    let text = text.into();
    let width = crate::paint::text_size(ui.ctx(), &text, TextRole::Small.font_id()).x;
    let gap = ui.spacing().item_spacing.x;
    let lead = ((ui.available_width() - width) / 2.0 - gap).max(0.0);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        hairline(ui, lead);
        caption(ui, text, ink::INK2);
        let rest = ui.available_width();
        hairline(ui, rest);
    });
    ui.add_space(4.0);
}

fn hairline(ui: &mut Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BORDER);
}
