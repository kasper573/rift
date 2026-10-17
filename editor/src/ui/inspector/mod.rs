pub mod bindings;
pub mod preview;
pub mod quests;
pub mod rows;
pub mod scripts;
pub mod settings;
pub mod usages;

use std::collections::BTreeMap;

use egui::{Align, Color32, Layout, RichText, Sense, Ui, Vec2};
use engine::core::content::{ContentRow, ModuleRule};
use engine::systems::area::world_map::WorldMapDef;
use serde_json::Value;

use self::bindings::bindings_section;
use self::rows::{fieldless_note, free_singles, show_rows};
use self::scripts::{
    is_objective, module_lines, outline_block, script_extras, script_summary, script_values,
    speaker_of, summary_of,
};
use self::usages::show_asset;
use super::{theme, widgets};
use crate::app::{
    BottomTab, Clipboard, ConfirmAction, ConfirmDialog, Editor, ModalDialog, PlacePick, RightTab,
    TextComparison, registry,
};
use crate::drafts::{clear_draft, draft_of, set_draft};
use crate::forms::{FormAction, FormChange, FormCx};
use crate::project::{ContentEdit, RowSelection, row_file};
use crate::rows::new_row_into;
use crate::schema;
use crate::tabs::EditorTab;
use crate::ui::browser::{self, BrowserAction};
use crate::value::RowValue;
use crate::views::area::{inspect, parts};
use crate::views::script::{PendingScriptAction, ScriptOwner};
use crate::views::{class, model, script, skill, tileset, vfx, world};
use crate::{forms, views};

const WIDE: f32 = 760.0;
pub(super) const LEFT_COLUMN: f32 = 390.0;
const SINGLE_COLUMN: f32 = 560.0;

pub fn show(editor: &mut Editor, ui: &mut Ui) {
    if editor.panels.bottom == BottomTab::History && editor.history_focus.is_some() {
        super::history::step_card(editor, ui);
        ui.add_space(8.0);
    }
    let selection = editor.selection.clone();
    if editor.project.is_none() {
        return;
    }
    if selection.is_game() {
        egui::ScrollArea::vertical()
            .id_salt("inspector-game")
            .show(ui, |ui| settings::show_game(editor, ui));
        return;
    }
    if selection.keys.is_empty() || selection.table == WorldMapDef::TABLE {
        if matches!(super::active_tab(editor), Some(EditorTab::World)) {
            world::inspect_parts(editor, ui, &selection);
            return;
        }
        ui.label(theme::muted(
            "Select a row, a module or a result to see it here.",
        ));
        return;
    }
    if selection.table == "asset" {
        show_asset(editor, ui, &selection.keys[0]);
        return;
    }
    let module_part = selection.parts.len() == 1
        && selection.parts[0]
            .strip_prefix("/modules/")
            .is_some_and(|rest| rest.parse::<usize>().is_ok())
        && !matches!(selection.table.as_str(), "area" | "blueprint");
    if !selection.parts.is_empty() && !module_part {
        views::inspect_parts(editor, ui, &selection);
        return;
    }
    if selection.keys.len() == 1 {
        let (table, key) = (selection.table.clone(), selection.keys[0].clone());
        if table == "script" {
            script::inspect_row(editor, ui, &key);
            return;
        }
        if let Some(EditorTab::Script(owner)) = super::active_tab(editor)
            && owner.is_row(&table, &key)
        {
            script::inspect_parts(editor, ui, &selection);
            return;
        }
        if table == "area"
            && matches!(super::active_tab(editor), Some(EditorTab::Area { key: open }) if open == key)
        {
            inspect::show(editor, ui, &key);
            return;
        }
        if table == "blueprint" {
            egui::ScrollArea::vertical()
                .id_salt(("inspector-blueprint", &key))
                .show(ui, |ui| {
                    parts::show_blueprint(editor, ui, &key, &[]);
                });
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("inspector", &table, &key))
            .show(ui, |ui| {
                row_form(editor, ui, &table, &key, false);
            });
    } else {
        egui::ScrollArea::vertical()
            .id_salt(("inspector-rows", &selection.table))
            .show(ui, |ui| {
                show_rows(editor, ui, &selection, false);
            });
    }
}

pub fn right_tabs(editor: &Editor) -> Vec<(RightTab, String)> {
    let selection = &editor.selection;
    let mut tabs = Vec::new();
    let shown_in_centre = selection.parts.is_empty()
        && match super::active_tab(editor) {
            Some(EditorTab::Row { table, key }) => {
                selection.keys.len() == 1 && table == selection.table && key == selection.keys[0]
            }
            Some(EditorTab::Rows { table, keys }) => {
                table == selection.table && keys == selection.keys
            }
            _ => false,
        };
    if selection.table == "input" && selection.parts.is_empty() && !selection.keys.is_empty() {
        tabs.push((RightTab::Conflicts, "Conflicts".to_owned()));
        if !shown_in_centre {
            tabs.push((RightTab::Inspector, "Inspector".to_owned()));
        }
    } else if has_preview(editor) {
        tabs.push((
            RightTab::Preview,
            if selection.table == "quest" {
                "Quest log".to_owned()
            } else {
                "Preview".to_owned()
            },
        ));
        if !shown_in_centre {
            tabs.push((RightTab::Inspector, "Inspector".to_owned()));
        }
    } else if !(shown_in_centre && views_with_own_form(&selection.table)) {
        tabs.push((RightTab::Inspector, "Inspector".to_owned()));
    }
    if !selection.is_game() {
        tabs.push((RightTab::UsedBy, "Used by".to_owned()));
    }
    tabs
}

/// The tables whose centre view lays the row's form out beside what it previews, so the panel
/// would only repeat it.
fn views_with_own_form(table: &str) -> bool {
    table == vfx::TABLE || table == vfx::SHADER_TABLE
}

pub fn has_preview(editor: &Editor) -> bool {
    let selection = &editor.selection;
    !selection.keys.is_empty()
        && selection.parts.is_empty()
        && matches!(
            selection.table.as_str(),
            "npc" | "player" | "model" | "item" | "quest" | "test_character"
        )
}

pub fn show_row_tab(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str) {
    egui::ScrollArea::vertical()
        .id_salt(("row-tab", table, key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            row_form(editor, ui, table, key, ui.available_width() >= WIDE);
        });
}

pub fn show_rows_tab(editor: &mut Editor, ui: &mut Ui, table: &str, keys: &[String]) {
    egui::ScrollArea::vertical()
        .id_salt(("rows-tab", table))
        .show(ui, |ui| {
            let selection = RowSelection::rows(table, keys.to_vec());
            show_rows(editor, ui, &selection, ui.available_width() >= WIDE);
        });
}

pub fn show_row(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str) {
    row_form(editor, ui, table, key, false);
}

fn row_form(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str, wide: bool) {
    let file = row_file(table, key);
    let Some(mut value) = editor.row_value(table, key) else {
        ui.label(theme::muted(format!("{file} is gone")));
        return;
    };
    let fixture = editor.shapes.is_fixture(table, key);
    let selected_module: Option<usize> = editor
        .selection
        .parts
        .first()
        .and_then(|part| part.strip_prefix("/modules/"))
        .and_then(|index| index.parse().ok());
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    let has_modules = root
        .get("properties")
        .and_then(|p| p.get("modules"))
        .is_some();
    let second_column = has_modules || table == "input";
    if fixture {
        editor.status.left.push(format!("{table}/{key}"));
        editor
            .status
            .left
            .push("fixture · a row every game has".to_owned());
    }
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    let mut module_ops: Vec<ModuleOp> = Vec::new();
    let mut status: Option<String> = None;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx =
            FormCx::new(project, root, shapes, form, &mut actions).owned_by(table, Some(key));
        cx.speaker = speaker_of(table, key);
        ui.push_id(("row", table, key), |ui| {
            if wide && second_column {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(LEFT_COLUMN);
                        changed |= row_card(ui, &mut cx, table, key, fixture, root, &mut value);
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width().min(420.0));
                        if table == "input" {
                            changed |= bindings_section(ui, &mut value);
                        } else {
                            changed |= modules_column(
                                ui,
                                &mut cx,
                                table,
                                &mut value,
                                &mut module_ops,
                                selected_module,
                                &mut status,
                            );
                        }
                    });
                });
            } else {
                if wide {
                    ui.set_max_width(SINGLE_COLUMN);
                }
                changed |= row_card(ui, &mut cx, table, key, fixture, root, &mut value);
                if table == "input" {
                    ui.add_space(10.0);
                    changed |= bindings_section(ui, &mut value);
                }
                if has_modules {
                    ui.add_space(10.0);
                    changed |= modules_column(
                        ui,
                        &mut cx,
                        table,
                        &mut value,
                        &mut module_ops,
                        selected_module,
                        &mut status,
                    );
                }
            }
        });
    }
    if let Some(status) = status {
        editor.status.left.push(format!("{table}/{key}"));
        editor.status.left.push(status);
    }
    if !editor.form.status.is_empty() {
        editor
            .status
            .left
            .push(format!("{table}/{key} › {}", editor.form.status.remove(0)));
        editor.status.left.append(&mut editor.form.status);
    }
    for op in module_ops {
        match op {
            ModuleOp::Copy(index) => {
                let module = value["modules"][index].clone();
                editor.clipboard = Some(Clipboard {
                    table: table.to_owned(),
                    from: key.to_owned(),
                    modules: vec![module],
                    origin: None,
                });
                editor.toast("Module copied");
            }
            ModuleOp::Paste => paste_modules(editor, table, key, &mut value, &mut changed),
        }
    }
    if changed.is_some() {
        let what = changed.0.clone().unwrap_or_default();
        let label = format!(
            "Edit {table}/{key}{}",
            if what.is_empty() {
                String::new()
            } else {
                format!(" {}", what.trim_start_matches('/'))
            }
        );
        let mut edit = ContentEdit::new(label);
        edit.write_json(&file, &value);
        editor.apply(edit);
    }
    for action in actions {
        act(editor, action, table, key);
    }
}

fn row_card(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    key: &str,
    fixture: bool,
    schema: &Value,
    value: &mut Value,
) -> FormChange {
    let mut changed = FormChange::default();
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(
            ui,
            &format!("{table}/{key}"),
            if fixture { "fixture" } else { "" },
            |ui| {
                if ui.button("Copy as JSON").clicked() {
                    ui.ctx()
                        .copy_text(serde_json::to_string_pretty(&*value).unwrap_or_default());
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    cx.actions
                        .push(FormAction::RowAction(BrowserAction::Duplicate));
                    ui.close();
                }
                if !fixture
                    && ui
                        .add(egui::Button::new("Rename key…").shortcut_text("F2"))
                        .clicked()
                {
                    cx.actions
                        .push(FormAction::RowAction(BrowserAction::Rename));
                    ui.close();
                }
                if ui.button("Show usages").clicked() {
                    cx.actions
                        .push(FormAction::RowAction(BrowserAction::Usages));
                    ui.close();
                }
                if !fixture {
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Delete…").shortcut_text("Del"))
                        .clicked()
                    {
                        cx.actions
                            .push(FormAction::RowAction(BrowserAction::Delete));
                        ui.close();
                    }
                }
            },
        );
        ui.add_space(4.0);
        forms::error_line(ui, cx, "");
        if table == model::TABLE {
            changed = model::inspect::row_body(ui, cx, value);
            return;
        }
        if table == tileset::TABLE {
            changed = tileset::inspect::row_body(ui, cx, value);
            return;
        }
        let hidden: &[&str] = match table {
            "input" => &["modules", "bindings"],
            class::TABLE => &["modules", "levels", "tree", "backdrop"],
            skill::TABLE => &["modules", "levels"],
            vfx::SHADER_TABLE => &["modules", "params"],
            _ => &["modules"],
        };
        changed = forms::object_without(ui, cx, schema, value, "", hidden);
        if table == class::TABLE {
            changed |= class::levels(ui, cx, value);
        }
        if table == skill::TABLE {
            changed |= skill::ranks(ui, cx, value);
        }
        if table == vfx::SHADER_TABLE {
            changed |= vfx::shader_params(ui, cx, value);
        }
    });
    changed
}

enum ModuleOp {
    Copy(usize),
    Paste,
}

fn modules_column(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    value: &mut Value,
    ops: &mut Vec<ModuleOp>,
    selected: Option<usize>,
    status: &mut Option<String>,
) -> FormChange {
    let variants = cx.shapes.module_variants(table);
    if !value["modules"].is_array() {
        value["modules"] = Value::Array(Vec::new());
    }
    let mut changed = FormChange::default();
    let scripts_only = table == "area";
    let count = value["modules"].as_array().map_or(0, Vec::len);
    let shown = if scripts_only {
        value["modules"].as_array().map_or(0, |list| {
            list.iter()
                .filter(|m| m["type"].as_str().is_some_and(|t| t.ends_with("_script")))
                .count()
        })
    } else {
        count
    };
    ui.horizontal(|ui| {
        widgets::section_count(
            ui,
            if scripts_only {
                "Area scripts"
            } else {
                "Modules"
            },
            shown,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = ui.add(
                egui::Button::new(RichText::new("+ Add module").color(Color32::WHITE))
                    .fill(theme::ACCENT)
                    .stroke(egui::Stroke::NONE),
            );
            let popup = egui::Popup::from_toggle_button_response(&button)
                .id(ui.id().with("add-module"))
                .width(330.0);
            popup.show(|ui| {
                if let Some(template) = add_module_popup(
                    ui,
                    cx,
                    table,
                    &variants,
                    value["modules"].as_array().expect("modules"),
                    scripts_only,
                    status,
                ) {
                    let list = value["modules"].as_array_mut().expect("modules");
                    list.push(template);
                    let path = format!("{table}/modules/{}", list.len() - 1);
                    cx.state.folded.remove(&path);
                    changed = FormChange::at("/modules");
                    ui.close();
                }
            });
            if ui
                .small_button("Paste")
                .on_hover_text("Paste copied modules")
                .clicked()
            {
                ops.push(ModuleOp::Paste);
            }
        });
    });
    let free = free_singles(cx, table, &variants, value);
    if !free.is_empty() {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::faint(format!("{} still free", free.join(", "))).size(12.0));
            });
        });
    }
    if scripts_only {
        module_counts(ui, value);
    }
    let mut remove = None;
    let mut swap = None;
    let mut duplicate = None;
    let mut objectives = 0;
    for index in 0..count {
        let path = format!("/modules/{index}");
        let module = &mut value["modules"][index];
        let tag = module.kind_or("module").to_owned();
        if scripts_only && !tag.ends_with("_script") {
            continue;
        }
        let is_script = tag.ends_with("_script");
        let summary = if is_script {
            script_summary(module)
        } else if table == "quest" && is_objective(&tag) {
            objectives += 1;
            format!("objective {objectives}")
        } else if let Some(condition) = module.get("condition") {
            let blocks = ["all", "any"]
                .iter()
                .find_map(|op| condition[op].as_array().map(Vec::len))
                .unwrap_or(1);
            format!(
                "condition · {blocks} {}",
                if blocks == 1 { "block" } else { "blocks" }
            )
        } else {
            summary_of(cx.root, module)
        };
        let folded_key = format!("{table}{path}");
        let folded = match (
            cx.state.folded.contains(&folded_key),
            cx.state.unfolded.contains(&folded_key),
        ) {
            (true, _) => true,
            (false, true) => false,
            (false, false) => is_script && selected != Some(index),
        };
        let is_selected = selected == Some(index);
        widgets::card_frame(is_selected).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let header = ui
                .horizontal(|ui| {
                    ui.label(
                        theme::strong(if is_script {
                            tag.replace('_', " ")
                        } else {
                            tag.clone()
                        })
                        .monospace(),
                    );
                    if folded || !is_script {
                        ui.add(
                            egui::Label::new(
                                RichText::new(summary.to_uppercase())
                                    .size(11.0)
                                    .color(theme::MUTED),
                            )
                            .truncate(),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.menu_button(theme::faint("…"), |ui| {
                            if ui.button("Copy").clicked() {
                                ops.push(ModuleOp::Copy(index));
                                ui.close();
                            }
                            if ui.button("Duplicate").clicked() {
                                duplicate = Some(index);
                                ui.close();
                            }
                            ui.separator();
                            if index > 0 && ui.button("Move up").clicked() {
                                swap = Some((index, index - 1));
                                ui.close();
                            }
                            if index + 1 < count && ui.button("Move down").clicked() {
                                swap = Some((index, index + 1));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Remove").clicked() {
                                remove = Some(index);
                                ui.close();
                            }
                        });
                        ui.label(theme::faint(if folded { "▸" } else { "▾" }).size(11.0));
                    });
                })
                .response
                .interact(Sense::click());
            if header.clicked() {
                if folded {
                    cx.state.folded.remove(&folded_key);
                    cx.state.unfolded.insert(folded_key.clone());
                } else {
                    cx.state.unfolded.remove(&folded_key);
                    cx.state.folded.insert(folded_key.clone());
                }
            }
            if !folded {
                changed |= card_body(ui, cx, table, &variants, module, &path);
            }
        });
        if !folded && tag == "inline_script" {
            outline_block(ui, cx, &value["modules"][index]);
        }
        ui.add_space(2.0);
    }
    let list = value["modules"].as_array_mut().expect("modules");
    if let Some(index) = remove {
        list.remove(index);
        changed = FormChange::at("/modules");
    }
    if let Some((a, b)) = swap {
        list.swap(a, b);
        changed = FormChange::at("/modules");
    }
    if let Some(index) = duplicate {
        let copy = list[index].clone();
        list.insert(index + 1, copy);
        changed = FormChange::at("/modules");
    }
    changed
}

pub(super) fn card_body(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    module: &mut Value,
    path: &str,
) -> FormChange {
    ui.add_space(4.0);
    let mut changed = FormChange::default();
    if let Some(variant) = schema::variant_of(variants, module) {
        let hidden: &[&str] = match variant.tag.as_str() {
            "inline_script" => &["nodes"],
            "shared_script" => &["values"],
            _ => &[],
        };
        let fieldless = variant.schema["properties"]
            .as_object()
            .is_none_or(|fields| {
                fields
                    .keys()
                    .all(|name| name == "type" || hidden.contains(&name.as_str()))
            });
        if fieldless {
            widgets::hint(ui, &fieldless_note(variant.schema));
        } else {
            changed |= forms::object_without(ui, cx, variant.schema, module, path, hidden);
        }
        if variant.tag == "shared_script" {
            changed |= script_values(ui, cx, module, path);
        }
    } else {
        changed |= forms::raw_json(ui, cx, module, path);
    }
    script_extras(ui, cx, table, module, path);
    changed
}

fn module_counts(ui: &mut Ui, value: &Value) {
    let mut counts: BTreeMap<String, usize> = Default::default();
    for module in value["modules"].as_array().into_iter().flatten() {
        let kind = module.kind_or("module");
        if kind.ends_with("_script") {
            continue;
        }
        *counts.entry(kind.to_owned()).or_default() += 1;
    }
    let text: Vec<String> = counts
        .iter()
        .map(|(kind, count)| format!("{count} {}", kind.replace('_', " ")))
        .collect();
    widgets::hint(
        ui,
        &format!(
            "Placed modules edit in the area view: {}.",
            if text.is_empty() {
                "none yet".to_owned()
            } else {
                text.join(", ")
            }
        ),
    );
}

pub(super) fn add_module_popup(
    ui: &mut Ui,
    cx: &mut FormCx,
    table: &str,
    variants: &[schema::SchemaVariant],
    list: &[Value],
    scripts_only: bool,
    status: &mut Option<String>,
) -> Option<Value> {
    let registry = registry();
    let id = ui.id().with("add-module-filter");
    let mut filter = draft_of(ui, id).unwrap_or_default();
    let response = widgets::search_box(ui, &mut filter, "filter", None);
    if response.changed() {
        set_draft(ui, id, &filter.clone());
    }
    widgets::default_focus(ui, &response);
    let needle = filter.to_lowercase();
    let mut picked = None;
    let mut blocked_count = 0;
    let mut offered = 0;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for variant in variants {
                if scripts_only && !variant.tag.ends_with("_script") {
                    continue;
                }
                offered += 1;
                let template = schema::variant_template(cx.root, variant);
                let rule = registry.module_rule(table, &template);
                let present = list
                    .iter()
                    .any(|module| module.kind() == variant.tag.as_str());
                let at_most_one = rule == Some(ModuleRule::AtMostOne);
                let blocked = present && at_most_one;
                if blocked {
                    blocked_count += 1;
                }
                let title = variant.tag.replace('_', " ");
                if !needle.is_empty() && !variant.tag.contains(&needle) && !title.contains(&needle)
                {
                    continue;
                }
                let description = variant.schema["description"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| match variant.tag.as_str() {
                        "inline_script" => "any number · nodes in this row".to_owned(),
                        "shared_script" => "any number · names a script row".to_owned(),
                        _ if at_most_one => "at most one".to_owned(),
                        _ => "any number".to_owned(),
                    });
                let line = if blocked {
                    "at most one · already added".to_owned()
                } else {
                    description
                };
                let full = ui.available_rect_before_wrap();
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(full.width(), 38.0),
                    if blocked {
                        Sense::hover()
                    } else {
                        Sense::click()
                    },
                );
                if response.hovered() && !blocked {
                    ui.painter().rect_filled(rect, 4.0, theme::ACCENT_SOFT);
                }
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(Vec2::new(6.0, 3.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                );
                if blocked {
                    child.add_enabled_ui(false, |ui| widgets::badge(ui, &variant.tag));
                } else {
                    widgets::badge(&mut child, &variant.tag);
                }
                child.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(title).monospace().color(if blocked {
                        theme::FAINT
                    } else {
                        theme::TEXT
                    }));
                    ui.label(RichText::new(line).size(11.5).color(if blocked {
                        theme::FAINT
                    } else {
                        theme::MUTED
                    }));
                });
                if response.clicked() {
                    picked = Some(template);
                }
            }
        });
    *status = Some(format!(
        "{offered} module types for {table}{}",
        if blocked_count > 0 {
            format!(" · {blocked_count} at most one, already used")
        } else {
            String::new()
        }
    ));
    if picked.is_some() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        clear_draft(ui, id);
        if picked.is_none() {
            ui.close();
        }
    }
    picked
}

pub fn with_row_form(
    editor: &mut Editor,
    table: &str,
    key: &str,
    show: impl FnOnce(&mut FormCx) -> bool,
) -> bool {
    let Some(root) = editor.row_value(table, key) else {
        return false;
    };
    let mut actions = Vec::new();
    let changed = {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let Some(project) = project.as_ref() else {
            return false;
        };
        let mut cx =
            FormCx::new(project, &root, shapes, form, &mut actions).owned_by(table, Some(key));
        show(&mut cx)
    };
    for action in actions {
        act(editor, action, table, key);
    }
    changed
}

pub fn act(editor: &mut Editor, action: FormAction, table: &str, key: &str) {
    match action {
        FormAction::OpenRow { table, key } => {
            editor.select(RowSelection::row(&table, &key));
            editor.open_tab(EditorTab::for_row(&table, &key));
        }
        FormAction::OpenScript { pointer } => {
            script::open_inline(editor, table, key, &pointer);
        }
        FormAction::MakeShared { pointer } => {
            script::open_inline(editor, table, key, &pointer);
            if let Some(owner) = ScriptOwner::inline(table, key, &pointer) {
                editor
                    .views
                    .script
                    .set_pending(&owner, PendingScriptAction::MakeShared);
            }
        }
        FormAction::MakeInline { script } => {
            let owner = ScriptOwner::Shared { key: script };
            editor.open_tab(EditorTab::Script(owner.clone()));
            editor
                .views
                .script
                .set_pending(&owner, PendingScriptAction::MakeInline);
        }
        FormAction::PickPlace { path } => {
            editor.pick = Some(PlacePick {
                file: row_file(table, key),
                pointer: path,
                area: None,
            });
            editor.toast(
                "Click a walkable cell on an area to pick the place (Alt-click for any point)",
            );
        }
        FormAction::NewRow {
            table: into_table,
            path,
        } => {
            new_row_into(editor, &into_table, Some((row_file(table, key), path)));
        }
        FormAction::RowAction(action) => browser::act(editor, table, key, action),
    }
}

fn paste_modules(
    editor: &mut Editor,
    table: &str,
    key: &str,
    value: &mut Value,
    changed: &mut FormChange,
) {
    let Some(clip) = editor.clipboard.clone() else {
        editor.toast("Nothing copied");
        return;
    };
    let from = clip.from;
    let registry = registry();
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(table);
    for module in &clip.modules {
        let tag = module.kind_or("module").to_owned();
        let Some(rule) = registry.module_rule(table, module) else {
            editor.toast(format!("a {table} row takes no {tag} module"));
            continue;
        };
        let list = value["modules"].as_array_mut().expect("modules");
        let existing = list
            .iter()
            .position(|known| known["type"] == module["type"]);
        match (rule, existing) {
            (ModuleRule::AtMostOne, Some(index)) => {
                editor.dialogs.push(ModalDialog::Confirm(ConfirmDialog {
                    title: format!("{table}/{key} already has {tag}"),
                    body: format!(
                        "A {table} has at most one {tag} module. Replace it with {from}'s?"
                    ),
                    label: format!("Replace {tag}"),
                    compare: Some(TextComparison {
                        left_title: format!("{key} now"),
                        left: module_lines(root, &list[index]),
                        right_title: format!("copied from {from}"),
                        right: module_lines(root, module),
                    }),
                    action: ConfirmAction::ReplaceModule {
                        table: table.to_owned(),
                        key: key.to_owned(),
                        index,
                        module: module.clone(),
                    },
                }));
            }
            _ => {
                list.push(module.clone());
                *changed = FormChange::at("/modules");
            }
        }
    }
}
