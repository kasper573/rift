use std::collections::BTreeMap;
use ui::controls;
use ui::egui;
use ui::form;
use ui::panel;
use ui::sheet;
use ui::theme;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use strum::VariantArray;
use ui::egui::Ui;
use ui::glyph::{self, Mark};

use crate::forms::{self, FormChange, FormCx};

const LABEL: f32 = 110.0;
const INPUT: f32 = 150.0;

/// A step of a row that grows by levels: a class's level, a skill's rank. The form of its type
/// edits its fields, and the cards the stats of kind `Stat` it adds.
pub trait Level: JsonSchema + Serialize + DeserializeOwned + 'static {
    type Stat: VariantArray + Serialize + Copy + Ord;
    /// What one level is called, capitalized.
    const NAME: &'static str;
    /// Where its stats are within it.
    const STATS: &'static str;
    fn stats(&mut self) -> &mut BTreeMap<Self::Stat, f32>;
    fn unit(stat: Self::Stat) -> Option<&'static str>;
    /// The level added after `last`.
    fn next(last: Option<&Self>) -> Self;
}

/// The cards of the levels of the row `value` read as its type, which `levels` finds at `path` in
/// it, each with a `note` beside its title, which sees every level in turn.
pub(crate) fn row_cards<Row: Serialize + DeserializeOwned, L: Level>(
    ui: &mut Ui,
    cx: &mut FormCx,
    value: &mut Value,
    path: &str,
    levels: fn(&mut Row) -> &mut Vec<L>,
    note: impl FnMut(usize, &L) -> Option<String>,
) -> FormChange {
    forms::of_row(ui, cx, value, path, |ui, cx, row: &mut Row| {
        cards(ui, cx, levels(row), path, note)
    })
}

fn cards<L: Level>(
    ui: &mut Ui,
    cx: &mut FormCx,
    levels: &mut Vec<L>,
    path: &str,
    mut note: impl FnMut(usize, &L) -> Option<String>,
) -> FormChange {
    let mut changed = FormChange::default();
    let noun = L::NAME.to_lowercase();
    form::section_count(ui, &format!("{noun}s"), levels.len());
    let mut remove = None;
    for (index, level) in levels.iter_mut().enumerate() {
        let note = note(index, level);
        ui.push_id(index, |ui| {
            if card(
                ui,
                cx,
                &format!("{path}/{index}"),
                index,
                note,
                level,
                &mut changed,
            ) {
                remove = Some(index);
            }
        });
        ui.add_space(6.0);
    }
    if let Some(index) = remove {
        levels.remove(index);
        changed = FormChange::at(path);
    }
    if ui.button(format!("+ {noun}")).clicked() {
        levels.push(L::next(levels.last()));
        changed = FormChange::at(path);
    }
    changed
}

fn card<L: Level>(
    ui: &mut Ui,
    cx: &mut FormCx,
    path: &str,
    index: usize,
    note: Option<String>,
    level: &mut L,
    changed: &mut FormChange,
) -> bool {
    let mut removed = false;
    panel::tile(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::strong(format!("{} {}", L::NAME, index + 1)));
            if let Some(note) = note {
                ui.label(theme::muted(note).size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                    .on_hover_text(format!("Remove this {}", L::NAME.to_lowercase()))
                    .clicked()
                {
                    removed = true;
                }
            });
        });
        ui.add_space(4.0);
        *changed |= forms::of_type_without(ui, cx, level, path, &[L::STATS]);
        sheet::section(ui, "stats");
        *changed |= stats(ui, cx, &format!("{path}{}", L::STATS), index, level);
    });
    removed
}

fn stats<L: Level>(
    ui: &mut Ui,
    cx: &mut FormCx,
    path: &str,
    index: usize,
    level: &mut L,
) -> FormChange {
    let mut changed = FormChange::default();
    let stats = level.stats();
    let mut unset = None;
    for (&stat, amount) in stats.iter_mut() {
        let key = stat_key(stat);
        field_row(ui, &key, |ui| {
            changed |= forms::quantity(ui, cx, amount, L::unit(stat), &format!("{path}/{key}"));
            if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                .on_hover_text(format!("Unset {key}"))
                .clicked()
            {
                unset = Some(stat);
            }
        });
    }
    if stats.is_empty() {
        let noun = L::NAME.to_lowercase();
        ui.label(theme::faint(format!("nothing changes at this {noun}")).size(12.5));
    }
    if let Some(stat) = unset {
        stats.remove(&stat);
        changed = FormChange::at(path);
    }
    let missing: Vec<L::Stat> = L::Stat::VARIANTS
        .iter()
        .copied()
        .filter(|stat| !stats.contains_key(stat))
        .collect();
    if !missing.is_empty() {
        let mut picked: Option<L::Stat> = None;
        controls::combo(("add-stat", index))
            .selected_text(theme::muted("+ stat"))
            .width(INPUT)
            .show_ui(ui, |ui| {
                for stat in missing {
                    ui.selectable_value(&mut picked, Some(stat), stat_key(stat));
                }
            });
        if let Some(stat) = picked {
            stats.insert(stat, 0.0);
            changed = FormChange::at(path);
        }
    }
    changed
}

fn field_row(ui: &mut Ui, label: &str, input: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(LABEL, ui.spacing().interact_size.y),
            egui::Sense::hover(),
        );
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        )
        .add(egui::Label::new(theme::muted(label).monospace().size(13.0)).truncate());
        ui.allocate_ui_with_layout(
            egui::vec2(INPUT, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(INPUT);
                input(ui);
            },
        );
    });
}

fn stat_key(stat: impl Serialize) -> String {
    serde_json::to_value(stat)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
