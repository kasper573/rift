use std::collections::BTreeMap;
use ui::chip;
use ui::egui;
use ui::form;
use ui::panel;
use ui::sheet;
use ui::theme;

use engine::core::content::serial::TypeTag;
use engine::core::content::{MODULES, ModuleRule, fields_file};
use engine::systems::registry;
use engine::systems::script::{ListedScriptModule, ScriptModule};
use serde_json::Value;
use ui::controls;
use ui::egui::Ui;

use super::{LEFT_COLUMN, act, add_module_popup};
use crate::app::Editor;
use crate::forms::{self, FormAction, FormCx};
use crate::project::{ContentEdit, RowSelection};
use crate::schema::{self, SchemaKind};

pub(super) fn show_rows(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection, wide: bool) {
    let table = selection.table.clone();
    let values: Vec<(String, Value)> = selection
        .keys
        .iter()
        .filter_map(|key| editor.fields_json(key).map(|value| (key.clone(), value)))
        .collect();
    let Some(reader) = editor.reader() else {
        return;
    };
    let mut modules: Vec<(String, Vec<Value>)> = Vec::new();
    let mut unread: Vec<(String, String)> = Vec::new();
    let holds_modules = registry().holds_modules(&table);
    for (key, value) in values.iter().filter(|_| holds_modules) {
        match reader.modules_in(&table, value) {
            Ok(list) => modules.push((key.clone(), list)),
            Err(error) => unread.push((key.clone(), error)),
        }
    }
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(&table);
    let mut module_edits: Vec<(String, Vec<Value>)> = Vec::new();
    let mut edits: Vec<(String, Value)> = Vec::new();
    let mut actions = Vec::new();
    let mut add_to_all: Option<Value> = None;
    let mut status: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for (key, _) in &values {
            let response = chip::plain(ui, key);
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
        let mut cx = FormCx::new(project, root, shapes, form, &mut actions).editing(&table, None);
        let shared_card = |ui: &mut Ui, cx: &mut FormCx, edits: &mut Vec<(String, Value)>| {
            panel::card(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::subtitled(ui, &format!("{} {}s", values.len(), table), "shared fields");
                ui.add_space(4.0);
                let mut rows = RowSet {
                    values: &values,
                    edits,
                };
                for (name, field) in schema::properties(root) {
                    if name == MODULES {
                        continue;
                    }
                    let resolved = schema::resolve(cx.root, field);
                    if matches!(schema::kind(cx.root, resolved), SchemaKind::Object)
                        && schema::marker(resolved).is_none()
                    {
                        sheet::section(ui, name);
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
                            module_edits: &mut Vec<(String, Vec<Value>)>,
                            add_to_all: &mut Option<Value>,
                            status: &mut Option<String>| {
            let variants = cx.shapes.module_variants(&table);
            let tags = |list: &[Value]| -> Vec<String> {
                list.iter().filter_map(TypeTag::of_json).collect()
            };
            let mut all: Option<Vec<String>> = None;
            let mut some: Vec<String> = Vec::new();
            for (_, list) in &modules {
                let mine = tags(list);
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
            sheet::section(ui, &format!("Modules in all {}", modules.len()));
            for (key, error) in &unread {
                ui.label(
                    theme::warn(format!("{key} is left out: its modules don't read")).size(12.5),
                )
                .on_hover_text(error);
            }
            for tag in &all {
                let firsts: Vec<(&String, usize, &Value)> = modules
                    .iter()
                    .filter_map(|(key, list)| {
                        let index = list
                            .iter()
                            .position(|m| TypeTag::of_json(m).as_ref() == Some(tag))?;
                        Some((key, index, &list[index]))
                    })
                    .collect();
                let agree = firsts.iter().all(|(_, _, module)| *module == firsts[0].2);
                panel::card(false).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    panel::title(ui, &tag.replace('_', " "));
                    ui.add_space(4.0);
                    if agree {
                        let mut shared = firsts[0].2.clone();
                        let path = format!("/{MODULES}/{tag}");
                        let changed = match schema::variant_of(&variants, &shared) {
                            Some(variant) => forms::object_without(
                                ui,
                                cx,
                                variant.schema,
                                &mut shared,
                                &path,
                                &[],
                            ),
                            None => forms::raw_json(ui, cx, &mut shared, &path),
                        };
                        if changed.is_some() {
                            for (key, index, _) in &firsts {
                                if let Some((_, list)) = modules.iter().find(|(k, _)| k == *key) {
                                    let mut next = list.clone();
                                    next[*index] = shared.clone();
                                    module_edits.push(((*key).clone(), next));
                                }
                            }
                        }
                    } else if tag == ScriptModule::TAG {
                        let reader = cx.project.reader();
                        let mut by_script: BTreeMap<String, Vec<String>> = BTreeMap::new();
                        for (key, _, module) in &firsts {
                            let script = reader
                                .from_json::<ListedScriptModule>(module)
                                .ok()
                                .and_then(|ListedScriptModule::Script(module)| {
                                    reader.path_of(module.script).map(str::to_owned)
                                })
                                .unwrap_or_else(|| "?".to_owned());
                            by_script.entry(script).or_default().push((*key).clone());
                        }
                        form::kv(ui, "script", |ui| {
                            ui.label(theme::muted(format!("{} values", by_script.len())).italics());
                        });
                        form::hint(
                            ui,
                            &by_script
                                .iter()
                                .map(|(script, keys)| format!("{script}: {}", keys.join(", ")))
                                .collect::<Vec<_>>()
                                .join(" · "),
                        );
                    } else {
                        form::hint(ui, "different in each row · open one row to edit its items");
                    }
                });
                ui.add_space(2.0);
            }
            if all.is_empty() {
                form::hint(ui, "none");
            }
            sheet::section(ui, "In some");
            let only_some: Vec<&String> = some.iter().filter(|tag| !all.contains(tag)).collect();
            if only_some.is_empty() {
                form::hint(ui, "none");
            } else {
                ui.horizontal_wrapped(|ui| {
                    for tag in only_some {
                        let holders = modules
                            .iter()
                            .filter(|(_, list)| tags(list).contains(tag))
                            .count();
                        chip::plain(ui, format!("{tag} · {holders} of {}", modules.len()));
                    }
                });
            }
            ui.add_space(6.0);
            let button = controls::dropdown(ui, format!("+ Add module to all {}", modules.len()));
            egui::Popup::from_toggle_button_response(&button)
                .id(ui.id().with("add-all"))
                .width(330.0)
                .show(|ui| {
                    if let Some(picked) = add_module_popup(ui, cx, &table, &variants, &[], status) {
                        *add_to_all = Some(picked.template);
                        ui.close();
                    }
                });
        };
        if !holds_modules {
            shared_card(ui, &mut cx, &mut edits);
        } else if wide {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(LEFT_COLUMN);
                    shared_card(ui, &mut cx, &mut edits);
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width().min(420.0));
                    modules_part(ui, &mut cx, &mut module_edits, &mut add_to_all, &mut status);
                });
            });
        } else {
            shared_card(ui, &mut cx, &mut edits);
            ui.add_space(8.0);
            modules_part(ui, &mut cx, &mut module_edits, &mut add_to_all, &mut status);
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
        let tag = TypeTag::of_json(&template).unwrap_or_default();
        let mut skipped: Vec<String> = Vec::new();
        for (key, list) in &modules {
            if !registry.takes_module(&table, list, &template) {
                skipped.push(key.clone());
                continue;
            }
            let mut next = list.clone();
            next.push(template.clone());
            module_edits.push((key.clone(), next));
        }
        if !skipped.is_empty() {
            editor.toast(format!(
                "{tag} skipped on {}: already there, at most one",
                skipped.join(", ")
            ));
        }
    }
    if !module_edits.is_empty() && !unread.is_empty() {
        let left_out: Vec<&str> = unread.iter().map(|(key, _)| key.as_str()).collect();
        editor.toast(format!(
            "Left out of the edit, their modules don't read: {}",
            left_out.join(", ")
        ));
    }
    for (key, list) in module_edits {
        let Some((_, value)) = values.iter().find(|(held, _)| *held == key) else {
            continue;
        };
        match reader.with_modules_in(&table, value, &list) {
            Ok(next) => edits.push((key, next)),
            Err(error) => editor.toast(format!("{key}: {error}")),
        }
    }
    if !edits.is_empty() {
        let mut edit = ContentEdit::new(format!("Edit {} {table} rows", edits.len()));
        for (key, value) in &edits {
            edit.write_json(fields_file(key), value);
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
            put(&mut next, path, shared.clone());
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
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            if !settable_apart(cx.root, field) {
                ui.label(
                    theme::faint(format!(
                        "{} values; edit the rows one by one",
                        distinct.len()
                    ))
                    .size(12.0),
                )
                .on_hover_text(listing.join("\n"));
                return;
            }
            ui.label(
                theme::faint(format!("{} values; an edit sets them all", distinct.len()))
                    .size(12.0),
            )
            .on_hover_text(listing.join("\n"));
            let mut shared = schema::template(cx.root, field, cx.nearby());
            if forms::value(ui, cx, field, &mut shared, path).is_some() {
                rows.set_all(path, &shared);
            }
        });
    });
}

/// Whether a field the rows hold apart can take one value for them all: a value with nothing
/// inside it, so setting it drops nothing a row held of its own.
fn settable_apart(root: &Value, field: &Value) -> bool {
    match schema::kind(root, field) {
        SchemaKind::Text
        | SchemaKind::Integer
        | SchemaKind::Number
        | SchemaKind::Boolean
        | SchemaKind::Enum(_)
        | SchemaKind::Const(_)
        | SchemaKind::Null => true,
        SchemaKind::Choice(options) => options
            .into_iter()
            .all(|option| settable_apart(root, option)),
        SchemaKind::Object
        | SchemaKind::Array(_)
        | SchemaKind::Map(_)
        | SchemaKind::Union(_)
        | SchemaKind::Any => false,
    }
}

/// Puts `value` at `pointer` in `json`, adding the fields on the way that a row leaves out.
fn put(json: &mut Value, pointer: &str, value: Value) {
    if let Some(at) = json.pointer_mut(pointer) {
        *at = value;
        return;
    }
    let Some((parent, key)) = pointer.rsplit_once('/') else {
        return;
    };
    if json.pointer(parent).is_none_or(Value::is_null) {
        put(json, parent, Value::Object(serde_json::Map::new()));
    }
    if let Some(Value::Object(map)) = json.pointer_mut(parent) {
        map.insert(key.to_owned(), value);
    }
}

pub(super) fn fieldless_note(variant: &Value) -> String {
    match schema::description(variant) {
        Some(description) => format!("No fields. {description}"),
        None => "No fields: having it is the setting.".to_owned(),
    }
}

pub(super) fn free_singles(
    cx: &FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    list: &[Value],
) -> Vec<String> {
    let registry = registry();
    variants
        .iter()
        .filter(|variant| {
            let template = schema::variant_template(cx.root, variant, cx.nearby());
            registry.module_rule(table, &template) == Some(ModuleRule::AtMostOne)
                && registry.takes_module(table, list, &template)
        })
        .map(|variant| variant.tag.replace('_', " "))
        .collect()
}
