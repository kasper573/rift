use std::collections::BTreeMap;

use egui::Ui;
use engine::systems::skill::SkillStat;
use serde_json::{Map, Value};
use strum::VariantArray;

use crate::forms::{self, FormChange, FormCx};
use crate::ui::{theme, widgets};

pub const TABLE: &str = "skill";

const LABEL: f32 = 110.0;
const INPUT: f32 = 150.0;

/// The rank cards of a skill row: each rank's point cost and the stats it adds, with the totals a
/// player at that rank sees.
pub fn ranks(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let mut changed = FormChange::default();
    if !value["levels"].is_array() {
        value["levels"] = Value::Array(Vec::new());
    }
    warnings(ui, value);
    let levels = value["levels"].as_array_mut().expect("array");
    widgets::section_count(ui, "ranks", levels.len());
    let mut remove = None;
    let mut totals: BTreeMap<String, f32> = BTreeMap::new();
    for (index, level) in levels.iter_mut().enumerate() {
        for (key, amount) in level["stats"].as_object().into_iter().flatten() {
            *totals.entry(key.clone()).or_default() += amount.as_f64().unwrap_or(0.0) as f32;
        }
        ui.push_id(index, |ui| {
            if rank_card(ui, cx, index, level, &totals, &mut changed) {
                remove = Some(index);
            }
        });
        ui.add_space(6.0);
    }
    if let Some(index) = remove {
        levels.remove(index);
        changed = FormChange::at("/levels");
    }
    if ui.button("+ rank").clicked() {
        levels.push(serde_json::json!({ "cost": 1, "stats": {} }));
        changed = FormChange::at("/levels");
    }
    changed
}

fn warnings(ui: &mut Ui, value: &Value) {
    if value["basic_attack"] != true {
        return;
    }
    let costs = value["cost"]
        .as_array()
        .is_some_and(|cost| !cost.is_empty());
    let cools = value["cooldown"].as_f64().unwrap_or(0.0) > 0.0;
    if costs {
        ui.label(theme::warn(
            "A basic attack spends nothing: a click on an enemy must always answer.",
        ));
    }
    if cools {
        ui.label(theme::warn(
            "A basic attack with a cooldown leaves clicks unanswered while it cools.",
        ));
    }
}

fn rank_card(
    ui: &mut Ui,
    cx: &mut FormCx,
    index: usize,
    level: &mut Value,
    totals: &BTreeMap<String, f32>,
    changed: &mut FormChange,
) -> bool {
    let path = format!("/levels/{index}");
    let mut removed = false;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::strong(format!("Rank {}", index + 1)));
            if !totals.is_empty() {
                let summed: Vec<String> = totals
                    .iter()
                    .map(|(key, total)| format!("{key} {total}"))
                    .collect();
                ui.label(theme::muted(format!("at this rank: {}", summed.join(" · "))).size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(egui::Sense::click()))
                    .on_hover_text("Remove this rank")
                    .clicked()
                {
                    removed = true;
                }
            });
        });
        ui.add_space(4.0);
        field_row(ui, "cost", |ui| {
            let schema = serde_json::json!({ "type": "integer", "minimum": 0 });
            if !level["cost"].is_number() {
                level["cost"] = Value::from(1);
            }
            *changed |= forms::number(
                ui,
                cx,
                &schema,
                &mut level["cost"],
                &format!("{path}/cost"),
                true,
            );
        });
        widgets::section(ui, "stats");
        if !level["stats"].is_object() {
            level["stats"] = Value::Object(Map::new());
        }
        let mut unset = None;
        let present: Vec<SkillStat> = SkillStat::VARIANTS
            .iter()
            .copied()
            .filter(|stat| level["stats"].get(stat_key(*stat)).is_some())
            .collect();
        for stat in &present {
            let key = stat_key(*stat);
            let stat_path = format!("{path}/stats/{key}");
            field_row(ui, &key, |ui| {
                let schema = match stat.unit() {
                    Some(unit) => serde_json::json!({ "type": "number", "x-unit": unit }),
                    None => serde_json::json!({ "type": "number" }),
                };
                *changed |= forms::number(
                    ui,
                    cx,
                    &schema,
                    &mut level["stats"][&key],
                    &stat_path,
                    false,
                );
                if ui
                    .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(egui::Sense::click()))
                    .on_hover_text(format!("Unset {key}"))
                    .clicked()
                {
                    unset = Some(key.clone());
                }
            });
        }
        if present.is_empty() {
            ui.label(theme::faint("nothing changes at this rank").size(12.5));
        }
        if let Some(key) = unset {
            level["stats"].as_object_mut().expect("object").remove(&key);
            *changed = FormChange::at(&format!("{path}/stats"));
        }
        let missing: Vec<SkillStat> = SkillStat::VARIANTS
            .iter()
            .copied()
            .filter(|stat| !present.contains(stat))
            .collect();
        if !missing.is_empty() {
            let mut picked: Option<SkillStat> = None;
            widgets::combo_box(("add-skill-stat", index))
                .selected_text(theme::muted("+ stat"))
                .width(INPUT)
                .show_ui(ui, |ui| {
                    for stat in missing {
                        ui.selectable_value(&mut picked, Some(stat), stat_key(stat));
                    }
                });
            if let Some(stat) = picked {
                level["stats"][stat_key(stat)] = Value::from(0.0);
                *changed = FormChange::at(&format!("{path}/stats"));
            }
        }
    });
    removed
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

fn stat_key(stat: SkillStat) -> String {
    serde_json::to_value(stat)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
