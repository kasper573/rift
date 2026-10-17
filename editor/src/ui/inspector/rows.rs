use std::collections::BTreeMap;

use egui::{RichText, Ui};
use engine::core::content::ModuleRule;
use serde_json::Value;

use super::{LEFT_COLUMN, act, add_module_popup};
use super::{theme, widgets};
use crate::app::{Editor, registry};
use crate::drafts::{clear_draft, draft_of, set_draft};
use crate::forms::{self, FormAction, FormCx};
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::schema::{self, SchemaKind};
use crate::value::RowValue;

pub(super) fn show_rows(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection, wide: bool) {
    let table = selection.table.clone();
    let values: Vec<(String, Value)> = selection
        .keys
        .iter()
        .filter_map(|key| {
            editor
                .row_value(&table, key)
                .map(|value| (key.clone(), value))
        })
        .collect();
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(&table);
    let mut edits: Vec<(String, Value)> = Vec::new();
    let mut actions = Vec::new();
    let mut add_to_all: Option<Value> = None;
    let mut status: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for (key, _) in &values {
            let response = widgets::chip(ui, key);
            if response.clicked() {
                actions.push(FormAction::OpenRow {
                    table: table.clone(),
                    key: key.clone(),
                });
            }
        }
    });
    ui.add_space(6.0);
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, root, shapes, form, &mut actions).owned_by(&table, None);
        let shared_card = |ui: &mut Ui, cx: &mut FormCx, edits: &mut Vec<(String, Value)>| {
            widgets::card_frame(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(
                    ui,
                    &format!("{} {}s", values.len(), table),
                    "shared fields",
                    |_| {},
                );
                ui.add_space(4.0);
                let mut rows = RowSet {
                    values: &values,
                    edits,
                };
                for (name, field) in schema::properties(root) {
                    if name == "modules" {
                        continue;
                    }
                    let resolved = schema::resolve(cx.root, field);
                    if matches!(schema::kind(cx.root, resolved), SchemaKind::Object)
                        && schema::marker(resolved).is_none()
                    {
                        widgets::section(ui, name);
                        for (sub_name, sub_field) in schema::properties(resolved) {
                            let path = format!("/{name}/{sub_name}");
                            shared_field(ui, cx, &mut rows, &path, sub_name, sub_field);
                        }
                        continue;
                    }
                    shared_field(ui, cx, &mut rows, &format!("/{name}"), name, field);
                }
            });
        };
        let modules_part = |ui: &mut Ui,
                            cx: &mut FormCx,
                            edits: &mut Vec<(String, Value)>,
                            add_to_all: &mut Option<Value>,
                            status: &mut Option<String>| {
            let variants = cx.shapes.module_variants(&table);
            let tags = |value: &Value| -> Vec<String> {
                value["modules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["type"].as_str().map(str::to_owned))
                    .collect()
            };
            let mut all: Option<Vec<String>> = None;
            let mut some: Vec<String> = Vec::new();
            for (_, value) in &values {
                let mine = tags(value);
                for tag in &mine {
                    if !some.contains(tag) {
                        some.push(tag.clone());
                    }
                }
                all = Some(match all {
                    None => mine,
                    Some(known) => known.into_iter().filter(|t| mine.contains(t)).collect(),
                });
            }
            let all = all.unwrap_or_default();
            widgets::section(ui, &format!("Modules in all {}", values.len()));
            for tag in &all {
                let firsts: Vec<(String, usize, Value)> = values
                    .iter()
                    .filter_map(|(key, value)| {
                        value["modules"].as_array().and_then(|list| {
                            list.iter()
                                .position(|m| m.kind() == tag)
                                .map(|index| (key.clone(), index, list[index].clone()))
                        })
                    })
                    .collect();
                let agree = firsts.iter().all(|(_, _, module)| *module == firsts[0].2);
                widgets::card_frame(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    widgets::card_header(ui, &tag.replace('_', " "), "", |_| {});
                    ui.add_space(4.0);
                    if agree {
                        let mut shared = firsts[0].2.clone();
                        let path = format!("/modules/{tag}");
                        let changed = match schema::variant_of(&variants, &shared) {
                            Some(variant) => forms::object_without(
                                ui,
                                cx,
                                variant.schema,
                                &mut shared,
                                &path,
                                &["nodes"],
                            ),
                            None => forms::raw_json(ui, cx, &mut shared, &path),
                        };
                        if changed.is_some() {
                            for (key, index, _) in &firsts {
                                if let Some((_, value)) = values.iter().find(|(k, _)| k == key) {
                                    let mut next = value.clone();
                                    next["modules"][*index] = shared.clone();
                                    edits.push((key.clone(), next));
                                }
                            }
                        }
                    } else if tag == "shared_script" {
                        let mut by_script: BTreeMap<String, Vec<String>> = BTreeMap::new();
                        for (key, _, module) in &firsts {
                            by_script
                                .entry(module.text_or("script", "?").to_owned())
                                .or_default()
                                .push(key.clone());
                        }
                        widgets::kv(ui, "script", |ui| {
                            ui.label(theme::muted(format!("{} values", by_script.len())).italics());
                        });
                        widgets::hint(
                            ui,
                            &by_script
                                .iter()
                                .map(|(script, keys)| format!("{script}: {}", keys.join(", ")))
                                .collect::<Vec<_>>()
                                .join(" · "),
                        );
                    } else {
                        widgets::hint(ui, "different in each row · open one row to edit its items");
                    }
                });
                ui.add_space(2.0);
            }
            if all.is_empty() {
                widgets::hint(ui, "none");
            }
            widgets::section(ui, "In some");
            let only_some: Vec<&String> = some.iter().filter(|tag| !all.contains(tag)).collect();
            if only_some.is_empty() {
                widgets::hint(ui, "none");
            } else {
                ui.horizontal_wrapped(|ui| {
                    for tag in only_some {
                        let holders = values
                            .iter()
                            .filter(|(_, value)| tags(value).contains(tag))
                            .count();
                        widgets::chip(ui, &format!("{tag} · {holders} of {}", values.len()));
                    }
                });
            }
            ui.add_space(6.0);
            let button = ui.button(format!("+ Add module to all {}", values.len()));
            egui::Popup::from_toggle_button_response(&button)
                .id(ui.id().with("add-all"))
                .width(330.0)
                .show(|ui| {
                    if let Some(template) =
                        add_module_popup(ui, cx, &table, &variants, &[], false, status)
                    {
                        *add_to_all = Some(template);
                        ui.close();
                    }
                });
        };
        if wide {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(LEFT_COLUMN);
                    shared_card(ui, &mut cx, &mut edits);
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width().min(420.0));
                    modules_part(ui, &mut cx, &mut edits, &mut add_to_all, &mut status);
                });
            });
        } else {
            shared_card(ui, &mut cx, &mut edits);
            ui.add_space(8.0);
            modules_part(ui, &mut cx, &mut edits, &mut add_to_all, &mut status);
        }
    }
    editor
        .status
        .left
        .push(format!("{} rows selected", values.len()));
    editor
        .status
        .left
        .push("an edit applies to every selected row".to_owned());
    if let Some(template) = add_to_all {
        let registry = registry();
        let tag = template.kind_or("module").to_owned();
        let mut skipped: Vec<String> = Vec::new();
        for (key, value) in &values {
            let present = value["modules"]
                .as_array()
                .is_some_and(|list| list.iter().any(|m| m["type"] == template["type"]));
            let blocked =
                present && registry.module_rule(&table, &template) == Some(ModuleRule::AtMostOne);
            if blocked {
                skipped.push(key.clone());
                continue;
            }
            let mut next = value.clone();
            if !next["modules"].is_array() {
                next["modules"] = Value::Array(Vec::new());
            }
            next["modules"]
                .as_array_mut()
                .expect("modules")
                .push(template.clone());
            edits.push((key.clone(), next));
        }
        if !skipped.is_empty() {
            editor.toast(format!(
                "{tag} skipped on {}: already there, at most one",
                skipped.join(", ")
            ));
        }
    }
    if !edits.is_empty() {
        let mut edit = ContentEdit::new(format!("Edit {} {table} rows", edits.len()));
        for (key, value) in &edits {
            edit.write_json(row_file(&table, key), value);
        }
        editor.apply(edit);
    }
    for action in actions {
        act(editor, action, &table, "");
    }
}

pub(super) struct RowSet<'a> {
    pub values: &'a [(String, Value)],
    pub edits: &'a mut Vec<(String, Value)>,
}

impl RowSet<'_> {
    fn distinct(&self, path: &str) -> Vec<Value> {
        let mut seen: Vec<Value> = Vec::new();
        for (_, value) in self.values {
            let v = value.pointer(path).cloned().unwrap_or(Value::Null);
            if !seen.contains(&v) {
                seen.push(v);
            }
        }
        seen
    }

    fn set_all(&mut self, path: &str, shared: &Value) {
        for (key, value) in self.values {
            let mut next = value.clone();
            if let Some(target) = next.pointer_mut(path) {
                *target = shared.clone();
            }
            self.edits.push((key.clone(), next));
        }
    }
}

pub(super) fn shared_field(
    ui: &mut Ui,
    cx: &mut FormCx,
    rows: &mut RowSet,
    path: &str,
    name: &str,
    field: &Value,
) {
    let distinct = rows.distinct(path);
    ui.horizontal(|ui| {
        forms::label_column(ui).label(theme::muted(name).monospace().size(13.0));
        if let [only] = distinct.as_slice() {
            let mut shared = only.clone();
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                if forms::value(ui, cx, field, &mut shared, path).is_some() {
                    rows.set_all(path, &shared);
                }
            });
            return;
        }
        let listing: Vec<String> = rows
            .values
            .iter()
            .map(|(key, value)| {
                format!(
                    "{key}: {}",
                    value
                        .pointer(path)
                        .map(|v| v
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()))
                        .unwrap_or_default()
                )
            })
            .collect();
        let id = ui.id().with(path).with("set-all");
        let mut draft = draft_of(ui, id).unwrap_or_default();
        let response = egui::Frame::new()
            .fill(theme::INPUT)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 12.0);
                ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .frame(egui::Frame::NONE)
                        .hint_text(RichText::new(format!("{} values", distinct.len())).italics())
                        .desired_width(f32::INFINITY),
                )
            });
        let response = response.inner.on_hover_text(listing.join("\n"));
        if response.changed() {
            set_draft(ui, id, &draft.clone());
        }
        if forms::committed(ui, &response) {
            clear_draft(ui, id);
            if !draft.trim().is_empty() {
                let parsed: Value = serde_json::from_str(draft.trim())
                    .unwrap_or(Value::String(draft.trim().to_owned()));
                rows.set_all(path, &parsed);
            }
        }
    });
}

pub(super) fn fieldless_note(variant: &Value) -> String {
    match variant["description"].as_str() {
        Some(description) => format!("No fields. {description}"),
        None => "No fields: having it is the setting.".to_owned(),
    }
}

pub(super) fn free_singles(
    cx: &FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    value: &Value,
) -> Vec<String> {
    let registry = registry();
    let list = value["modules"].as_array().cloned().unwrap_or_default();
    variants
        .iter()
        .filter(|variant| {
            let template = schema::variant_template(cx.root, variant);
            registry.module_rule(table, &template) == Some(ModuleRule::AtMostOne)
                && !list
                    .iter()
                    .any(|module| module.kind() == variant.tag.as_str())
        })
        .map(|variant| variant.tag.replace('_', " "))
        .collect()
}
