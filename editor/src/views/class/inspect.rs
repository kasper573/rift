use egui::{Sense, Stroke, Ui, Vec2};
use serde_json::{Map, Value};

use super::tree::{self, CarriedNode, Nodes};
use super::{TABLE, node_of, select_node, write};
use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::project::RowSelection;
use crate::ui::{inspector, theme, widgets};
use crate::{condition, picker, schema};

const DROP_HEIGHT: f32 = 30.0;

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(node) = node_of(selection).map(str::to_owned) else {
        return;
    };
    let key = selection.keys[0].clone();
    let Some(mut row) = editor.row_value(TABLE, &key) else {
        return;
    };
    if !row["tree"][&node].is_object() {
        ui.label(theme::muted(format!("node {node} is gone")));
        return;
    }
    let nodes = tree::nodes_of(&row);
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(TABLE);
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    let mut removed = false;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx =
            FormCx::new(project, root, shapes, form, &mut actions).owned_by(TABLE, Some(&key));
        egui::ScrollArea::vertical()
            .id_salt(("class-node", &key, &node))
            .show(ui, |ui| {
                ui.label(
                    theme::mono(format!("{TABLE}/{key} › node {node}"))
                        .color(theme::CODE)
                        .size(12.0),
                );
                ui.add_space(6.0);
                changed |= fields(ui, &mut cx, &mut row, &node);
                widgets::section(ui, "access requirements");
                changed |= requirements(ui, &mut cx, &mut row, &nodes, &node);
                ui.add_space(10.0);
                if widgets::danger(ui, "Remove node").clicked() {
                    removed = true;
                }
            });
    }
    if removed {
        tree::remove_node(&mut row, &node);
        write(editor, &key, "Remove node", &row);
        select_node(editor, &key, None);
    } else if changed.is_some() {
        write(editor, &key, "Edit node", &row);
    }
    for action in actions {
        inspector::act(editor, action, TABLE, &key);
    }
}

fn fields(ui: &mut Ui, cx: &mut FormCx, row: &mut Value, node: &str) -> FormChange {
    let mut changed = FormChange::default();
    let path = format!("{}{node}", super::TREE_PART);
    widgets::kv(ui, "skill", |ui| {
        let mut skill = row["tree"][node]["skill"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if picker::row(ui, cx, "skill", &mut skill, &format!("{path}/skill")) {
            row["tree"][node]["skill"] = Value::String(skill);
            changed = FormChange::at(&path);
        }
    });
    for axis in ["x", "y"] {
        widgets::kv(ui, axis, |ui| {
            let held = row["tree"][node]["at"][axis].as_f64().unwrap_or(0.0) as f32;
            let mut draft = held;
            let response = forms::boxed_input(ui, None, |ui| {
                ui.add(
                    egui::DragValue::new(&mut draft)
                        .speed(1.0)
                        .range(0.0..=f32::MAX),
                )
            });
            if response.changed() && draft != held {
                row["tree"][node]["at"][axis] = Value::from(draft);
                changed = FormChange::at(&path);
            }
        });
    }
    changed
}

fn requirements(
    ui: &mut Ui,
    cx: &mut FormCx,
    row: &mut Value,
    nodes: &Nodes,
    node: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    let path = format!("{}{node}/requires", super::TREE_PART);
    if !row["tree"][node]["requires"].is_array() {
        row["tree"][node]["requires"] = Value::Array(Vec::new());
    }
    let count = row["tree"][node]["requires"].as_array().map_or(0, Vec::len);
    let level_count = row["levels"].as_array().map_or(1, Vec::len).max(1) as u32;
    let mut remove = None;
    for index in 0..count {
        let item_path = format!("{path}/{index}");
        let kind = row["tree"][node]["requires"][index]["type"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        widgets::card_frame(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::badge(ui, &kind);
                ui.label(theme::strong(schema::title(&kind)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Label::new(theme::faint("✕").size(11.0)).sense(Sense::click()))
                        .on_hover_text("Remove this requirement")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            });
            let schema_root = cx.root;
            let item = &mut row["tree"][node]["requires"][index];
            match kind.as_str() {
                "skill" => {
                    let named = item["node"].as_str().unwrap_or_default().to_owned();
                    widgets::kv(ui, "node", |ui| {
                        let mut picked = named.clone();
                        widgets::combo_box(("requirement-node", index))
                            .width(ui.available_width() - 8.0)
                            .selected_text(if named.is_empty() {
                                theme::muted("pick a node…")
                            } else {
                                theme::mono(&named).size(13.0)
                            })
                            .show_ui(ui, |ui| {
                                for other in nodes.keys().filter(|other| *other != node) {
                                    let loops = tree::would_cycle(nodes, node, other);
                                    let response = ui.add_enabled(
                                        !loops,
                                        egui::Button::selectable(picked == *other, other),
                                    );
                                    if loops {
                                        response.on_disabled_hover_text(format!(
                                            "{other} already needs {node}: picking it would close a loop"
                                        ));
                                    } else if response.clicked() {
                                        picked = other.clone();
                                    }
                                }
                            });
                        if picked != named {
                            item["node"] = Value::String(picked);
                            changed = FormChange::at(&item_path);
                        }
                    });
                    let ranks = nodes
                        .get(&named)
                        .map_or(1, |named| tree::skill_ranks(cx.files, &named.skill));
                    widgets::kv(ui, "rank", |ui| {
                        let held = item["rank"].as_u64().unwrap_or(1) as u8;
                        let mut draft = held.min(ranks);
                        let response =
                            forms::boxed_input(ui, Some(&format!("of {ranks}")), |ui| {
                                ui.add(egui::DragValue::new(&mut draft).range(1..=ranks))
                            });
                        if (response.changed() && draft != held) || draft != held {
                            item["rank"] = Value::from(draft);
                            changed = FormChange::at(&item_path);
                        }
                    });
                }
                "level" => {
                    widgets::kv(ui, "level", |ui| {
                        let held = item["level"].as_u64().unwrap_or(1) as u32;
                        let mut draft = held;
                        let response =
                            forms::boxed_input(ui, Some(&format!("of {level_count}")), |ui| {
                                ui.add(egui::DragValue::new(&mut draft).range(1..=level_count))
                            });
                        if response.changed() && draft != held {
                            item["level"] = Value::from(draft);
                            changed = FormChange::at(&item_path);
                        }
                    });
                }
                "condition" => {
                    let condition_path = format!("{item_path}/condition");
                    let schema = schema::at_pointer(schema_root, schema_root, row, &condition_path)
                        .cloned()
                        .unwrap_or(Value::Null);
                    let item = &mut row["tree"][node]["requires"][index];
                    if item["condition"].is_null() {
                        item["condition"] = Value::Object(Map::new());
                    }
                    changed |= condition::edit(ui, cx, &schema, &mut item["condition"], &condition_path);
                }
                _ => {
                    ui.label(theme::warn(format!("unknown kind {kind:?}")));
                }
            }
        });
        ui.add_space(4.0);
    }
    if count == 0 {
        ui.label(theme::faint("a root: available from the start").size(12.5));
    }
    if let Some(index) = remove {
        row["tree"][node]["requires"]
            .as_array_mut()
            .expect("array")
            .remove(index);
        changed = FormChange::at(&path);
    }
    ui.add_space(4.0);
    ui.menu_button("+ add requirement", |ui| {
        let kinds = [
            (
                "skill",
                serde_json::json!({ "type": "skill", "node": "", "rank": 1 }),
            ),
            ("level", serde_json::json!({ "type": "level", "level": 2 })),
            (
                "condition",
                serde_json::json!({ "type": "condition", "condition": {} }),
            ),
        ];
        for (kind, template) in kinds {
            if ui.button(schema::title(kind)).clicked() {
                row["tree"][node]["requires"]
                    .as_array_mut()
                    .expect("array")
                    .push(template);
                changed = FormChange::at(&path);
                ui.close();
            }
        }
    });
    if egui::DragAndDrop::has_payload_of_type::<CarriedNode>(ui.ctx()) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), DROP_HEIGHT), Sense::hover());
        let lit = response.contains_pointer();
        widgets::dashed_rect(
            ui.painter(),
            rect,
            Stroke::new(1.0, if lit { theme::ACCENT } else { theme::BORDER }),
            5.0,
            3.0,
        );
        let carried = egui::DragAndDrop::payload::<CarriedNode>(ui.ctx())
            .map(|carried| carried.0.clone())
            .unwrap_or_default();
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("drop to require {carried} at rank 1"),
            egui::FontId::proportional(12.5),
            if lit { theme::TEXT } else { theme::MUTED },
        );
        if let Some(dropped) = response.dnd_release_payload::<CarriedNode>() {
            let named = dropped.0.clone();
            let listed = row["tree"][node]["requires"]
                .as_array()
                .is_some_and(|list| list.iter().any(|item| item["node"] == named));
            if named == node {
                cx.state.status = vec!["a node can't require itself".to_owned()];
            } else if listed {
                cx.state.status = vec![format!("{named} is already required")];
            } else if tree::would_cycle(nodes, node, &named) {
                cx.state.status = vec![format!("{named} already needs {node}")];
            } else {
                row["tree"][node]["requires"]
                    .as_array_mut()
                    .expect("array")
                    .push(serde_json::json!({ "type": "skill", "node": named, "rank": 1 }));
                changed = FormChange::at(&path);
            }
        }
    }
    if row["tree"][node]["requires"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        row["tree"][node]
            .as_object_mut()
            .expect("object")
            .remove("requires");
    }
    changed
}
