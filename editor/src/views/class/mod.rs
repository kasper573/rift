pub mod inspect;
pub mod tree;

use std::collections::HashMap;

use egui::Ui;
use engine::systems::stat::StatKind;
use serde_json::{Map, Value};
use strum::VariantArray;

use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::ui::{inspector, theme, widgets};

pub const TABLE: &str = "class";
pub const TREE_PART: &str = "/tree/";

const LABEL: f32 = 110.0;
const INPUT: f32 = 150.0;

#[derive(Default)]
pub struct ClassViews {
    pub pages: HashMap<String, ClassPage>,
    pub snap: Option<bool>,
    pub drag: Option<tree::NodeDrag>,
    pub frame_drag: Option<tree::FrameDrag>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ClassPage {
    #[default]
    Levels,
    Tree,
}

impl ClassViews {
    pub fn page(&self, key: &str) -> ClassPage {
        self.pages.get(key).copied().unwrap_or_default()
    }

    pub fn snaps(&self) -> bool {
        self.snap.unwrap_or(true)
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    if editor.row_value(TABLE, key).is_none() {
        ui.label(theme::muted(format!("{} is gone", row_file(TABLE, key))));
        return;
    }
    let page = editor.views.class.page(key);
    ui.horizontal(|ui| {
        for (target, label) in [(ClassPage::Levels, "levels"), (ClassPage::Tree, "tree")] {
            if widgets::mode_chip(ui, label, page == target).clicked() {
                editor.views.class.pages.insert(key.to_owned(), target);
                if target == ClassPage::Levels {
                    editor.select(RowSelection::row(TABLE, key));
                }
            }
        }
    });
    ui.add_space(4.0);
    match page {
        ClassPage::Levels => inspector::show_row_tab(editor, ui, TABLE, key),
        ClassPage::Tree => tree::show(editor, ui, key),
    }
}

pub fn node_of(selection: &RowSelection) -> Option<&str> {
    if selection.table != TABLE || selection.keys.len() != 1 {
        return None;
    }
    selection
        .parts
        .first()?
        .strip_prefix(TREE_PART)
        .filter(|node| !node.is_empty())
}

pub fn select_node(editor: &mut Editor, key: &str, node: Option<&str>) {
    match node {
        Some(node) => editor.select(RowSelection::parts(
            TABLE,
            key,
            vec![format!("{TREE_PART}{node}")],
        )),
        None => editor.select(RowSelection::row(TABLE, key)),
    }
}

pub fn write(editor: &mut Editor, key: &str, label: &str, value: &Value) {
    let mut edit = ContentEdit::new(format!("{label} · {TABLE}/{key}"));
    edit.write_json(row_file(TABLE, key), value);
    editor.apply(edit);
}

pub fn levels(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let mut changed = FormChange::default();
    if !value["levels"].is_array() {
        value["levels"] = Value::Array(Vec::new());
    }
    let levels = value["levels"].as_array_mut().expect("array");
    let mut remove = None;
    widgets::section_count(ui, "levels", levels.len());
    for (index, level) in levels.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            if level_card(ui, cx, index, level, &mut changed) {
                remove = Some(index);
            }
        });
        ui.add_space(6.0);
    }
    if let Some(index) = remove {
        levels.remove(index);
        changed = FormChange::at("/levels");
    }
    if ui.button("+ level").clicked() {
        let xp = levels
            .last()
            .and_then(|level| level["xp"].as_u64())
            .map_or(0, |xp| xp + 100);
        levels.push(serde_json::json!({ "xp": xp, "stats": {}, "skill_points": 1 }));
        changed = FormChange::at("/levels");
    }
    changed
}

fn level_card(
    ui: &mut Ui,
    cx: &mut FormCx,
    index: usize,
    level: &mut Value,
    changed: &mut FormChange,
) -> bool {
    let path = format!("/levels/{index}");
    let mut removed = false;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::strong(format!("Level {}", index + 1)));
            let xp = level["xp"].as_u64().unwrap_or(0);
            ui.label(theme::muted(if index == 0 && xp == 0 {
                "the start".to_owned()
            } else {
                format!("from {xp} xp")
            }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(egui::Sense::click()))
                    .on_hover_text("Remove this level")
                    .clicked()
                {
                    removed = true;
                }
            });
        });
        ui.add_space(4.0);
        field_row(ui, "xp", |ui| {
            let schema = serde_json::json!({ "type": "integer", "minimum": 0 });
            *changed |= forms::number(
                ui,
                cx,
                &schema,
                &mut level["xp"],
                &format!("{path}/xp"),
                true,
            );
        });
        field_row(ui, "skill points", |ui| {
            let schema = serde_json::json!({ "type": "integer", "minimum": 0 });
            if !level["skill_points"].is_number() {
                level["skill_points"] = Value::from(0);
            }
            *changed |= forms::number(
                ui,
                cx,
                &schema,
                &mut level["skill_points"],
                &format!("{path}/skill_points"),
                true,
            );
        });
        widgets::section(ui, "stats");
        if !level["stats"].is_object() {
            level["stats"] = Value::Object(Map::new());
        }
        let mut unset = None;
        let present: Vec<StatKind> = StatKind::VARIANTS
            .iter()
            .copied()
            .filter(|kind| level["stats"].get(stat_key(*kind)).is_some())
            .collect();
        for kind in &present {
            let key = stat_key(*kind);
            let stat_path = format!("{path}/stats/{key}");
            field_row(ui, &key, |ui| {
                let schema = match kind.unit() {
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
            ui.label(theme::faint("nothing changes at this level").size(12.5));
        }
        if let Some(key) = unset {
            level["stats"].as_object_mut().expect("object").remove(&key);
            *changed = FormChange::at(&format!("{path}/stats"));
        }
        let missing: Vec<StatKind> = StatKind::VARIANTS
            .iter()
            .copied()
            .filter(|kind| !present.contains(kind))
            .collect();
        if !missing.is_empty() {
            let mut picked: Option<StatKind> = None;
            widgets::combo_box(("add-stat", index))
                .selected_text(theme::muted("+ stat"))
                .width(INPUT)
                .show_ui(ui, |ui| {
                    for kind in missing {
                        ui.selectable_value(&mut picked, Some(kind), stat_key(kind));
                    }
                });
            if let Some(kind) = picked {
                level["stats"][stat_key(kind)] = Value::from(0.0);
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

fn stat_key(kind: StatKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
