use std::collections::{BTreeMap, BTreeSet};

use egui::{RichText, Ui};
use engine::core::script::{OwnerKindDef, VariableType};
use engine::systems::area::blueprint::holes;
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::script::OwnerKind;
use serde_json::{Map, Value};
use strum::VariantArray;

use super::nodes::{self, LineSpeaker};
use super::{RenameNodeDialog, ScriptDoc, ScriptOwner, ScriptPath, ScriptRef, select_nodes, write};
use crate::app::{BottomTab, Editor, ModalDialog};
use crate::forms::{FormAction, FormChange, FormCx};
use crate::project::{ContentEdit, RowSelection, catalogue_of, row_file, row_of};
use crate::tabs::EditorTab;
use crate::ui::{dialogs, inspector, theme, widgets};
use crate::value::RowValue;
use crate::views::area::holes::fill_holes;
use crate::{condition, forms, picker, rows, schema, spans, thumbs};

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(owner) = editor.views.script.focused() else {
        ui.label(theme::muted("No script in view"));
        return;
    };
    let Some(doc) = ScriptDoc::load(editor, &owner) else {
        ui.label(theme::muted(format!("{} is gone", owner.title())));
        return;
    };
    super::sync_with_content(editor);
    let names: Vec<String> = selection
        .parts
        .iter()
        .filter_map(|part| part.strip_prefix("node:").map(str::to_owned))
        .filter(|name| doc.nodes().contains_key(name))
        .collect();
    let choice: Option<(String, usize)> = selection
        .parts
        .iter()
        .find_map(|part| part.strip_prefix("choice:"))
        .and_then(|rest| rest.rsplit_once('#'))
        .and_then(|(name, index)| Some((name.to_owned(), index.parse().ok()?)));
    egui::ScrollArea::vertical()
        .id_salt(("script-inspector", owner.slug()))
        .show(ui, |ui| match names.as_slice() {
            [] => script_inspector(editor, ui, &doc),
            [name] => {
                let name = name.clone();
                match &choice {
                    Some((node, index)) if *node == name => {
                        choice_inspector(editor, ui, &doc, &name, *index)
                    }
                    _ => node_inspector(editor, ui, &doc, &name),
                }
            }
            many => {
                breadcrumb(
                    ui,
                    &format!("{} › {} nodes", owner_short(editor, &doc), many.len()),
                );
                widgets::card_frame(true).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    widgets::card_title(ui, &format!("{} nodes", many.len()));
                    for name in many {
                        if let Some(node) = doc.nodes().get(name) {
                            let summary =
                                nodes::summary(node, catalogue_of(editor.project.as_ref()));
                            ui.horizontal(|ui| {
                                ui.label(theme::mono(name).size(12.5));
                                ui.label(theme::muted(widgets::clip(&summary, 50)).size(12.5));
                            });
                        }
                    }
                    ui.add_space(6.0);
                    if widgets::danger(ui, "Delete these nodes").clicked() {
                        super::delete_nodes(editor, &doc, many.to_vec());
                    }
                });
            }
        });
}

fn breadcrumb(ui: &mut Ui, text: &str) {
    ui.label(theme::muted(text).monospace().size(13.0));
    ui.add_space(4.0);
}

fn owner_short(editor: &Editor, doc: &ScriptDoc) -> String {
    let label = super::owner_label(editor, doc);
    match label.rsplit_once(" · object ") {
        Some((_, object)) => object.to_owned(),
        None => label,
    }
}

pub fn script_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    super::sync_with_content(editor);
    if super::run::inspector(editor, ui, doc) {
        return;
    }
    if doc.is_shared() {
        shared_inspector(editor, ui, doc);
    } else {
        inline_inspector(editor, ui, doc);
    }
}

pub fn names_of(map: &Value) -> String {
    let names: Vec<&str> = map
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.as_str())
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(" · ")
    }
}

fn nodes_line(doc: &ScriptDoc) -> String {
    let count = doc.nodes().len();
    let dialogue = doc
        .nodes()
        .values()
        .filter(|node| nodes::is_dialogue(node))
        .count();
    if dialogue > 0 {
        format!("{count} · {dialogue} dialogue")
    } else {
        count.to_string()
    }
}

fn inline_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let owner_key = doc.owner.row_label();
    breadcrumb(ui, &format!("{} › inline script", owner_short(editor, doc)));
    let mut open_owner = false;
    let mut make = false;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, "inline script", "in its owner's row", |ui| {
            if ui.button("Open the owner's row").clicked() {
                open_owner = true;
                ui.close();
            }
            if ui.button("Make shared…").clicked() {
                make = true;
                ui.close();
            }
        });
        widgets::kv(ui, "owner", |ui| {
            if ui
                .add(
                    egui::Label::new(theme::mono(&owner_key).size(13.0))
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("Open the row")
                .clicked()
            {
                open_owner = true;
            }
        });
        widgets::kv_text(ui, "state", names_of(&doc.script["state"]));
        widgets::kv_text(ui, "locals", names_of(&doc.script["locals"]));
        widgets::kv_text(ui, "nodes", nodes_line(doc));
    });
    events_section(editor, ui, doc);
    other_scripts_section(editor, ui, doc);
    owner_section(editor, ui, doc);
    action_buttons(editor, ui, doc);
    if open_owner {
        editor.select(RowSelection::row(doc.owner.table(), doc.owner.key()));
        editor.open_tab(EditorTab::for_row(doc.owner.table(), doc.owner.key()));
    }
    if make {
        make_shared_dialog(editor, doc);
    }
}

fn shared_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let key = doc.owner.key().to_owned();
    breadcrumb(ui, &format!("script/{key}"));
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of("script");
    let owner_schema = root
        .pointer("/properties/owner")
        .cloned()
        .unwrap_or(Value::Null);
    let mut script = doc.script.clone();
    let mut actions = Vec::new();
    let mut make = false;
    let changed = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        widgets::card_frame(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, &key, "shared script", |ui| {
                    if ui.button("Make inline…").clicked() {
                        make = true;
                        ui.close();
                    }
                });
                let mut changed = FormChange::default();
                widgets::kv(ui, "owner", |ui| {
                    let mut owner = script["owner"].clone();
                    if forms::value(ui, &mut cx, &owner_schema, &mut owner, "/owner").is_some() {
                        script["owner"] = owner;
                        changed = FormChange::at("/owner");
                    }
                });
                let variables = script["variables"].as_object().map_or(0, |v| v.len());
                widgets::kv_text(
                    ui,
                    "variables",
                    if variables == 0 {
                        "none".to_owned()
                    } else {
                        variables.to_string()
                    },
                );
                if script["state"].as_object().is_some_and(|m| !m.is_empty()) {
                    widgets::kv_text(ui, "state", names_of(&script["state"]));
                }
                widgets::kv_text(ui, "locals", names_of(&script["locals"]));
                let returns = doc.nodes().values().any(|node| {
                    node.kind() == "return" && node.get("value").is_some_and(|v| !v.is_null())
                });
                widgets::kv_text(ui, "returns", if returns { "a value" } else { "nothing" });
                widgets::kv_text(ui, "nodes", nodes_line(doc));
                changed
            })
            .inner
    };
    for action in actions {
        act(editor, action, doc);
    }
    if changed.is_some() {
        write(editor, doc, script, format!("Edit script/{key}"));
    }
    variables_section(editor, ui, doc);
    run_by_section(editor, ui, doc);
    action_buttons(editor, ui, doc);
    if make {
        make_inline_dialog(editor, doc);
    }
}

fn events_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    widgets::section(ui, "Events");
    let mut go: Option<String> = None;
    let mut any = false;
    for (name, node) in doc.nodes() {
        if !nodes::is_event(node) {
            continue;
        }
        any = true;
        let event = nodes::event_of(node).unwrap_or("?").to_owned();
        let pieces = nodes::pieces(node, catalogue_of(editor.project.as_ref()));
        let detail = pieces
            .first()
            .map(|line| line.iter().map(|p| p.text.clone()).collect::<String>())
            .unwrap_or_default();
        let response = ui
            .horizontal(|ui| {
                ui.label(theme::faint("▸").size(12.0));
                ui.label(theme::strong(&event).monospace().size(13.0));
                ui.label(
                    theme::muted(detail.replace(" tiles", ""))
                        .monospace()
                        .size(12.5),
                );
            })
            .response
            .interact(egui::Sense::click());
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            go = Some(name.clone());
        }
    }
    if !any {
        widgets::hint(
            ui,
            "No event yet: Tab opens the palette with the events this owner answers.",
        );
    }
    if let Some(name) = go {
        select_nodes(editor, doc, &[name]);
    }
}

fn other_scripts_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let ScriptOwner::Inline { table, key, module } = &doc.owner else {
        return;
    };
    let Some(row) = editor.row_value(table, key) else {
        return;
    };
    let display = row["name"]
        .as_str()
        .or(row["title"].as_str())
        .or(row["label"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| key.clone());
    let possessive = if display.ends_with('s') {
        format!("{display}'")
    } else {
        format!("{display}'s")
    };
    widgets::section(ui, &format!("{possessive} other scripts"));
    let (holder, scripts_field) = match module.object() {
        Some(index) => (row["modules"][index].clone(), "scripts"),
        None => (row.clone(), "modules"),
    };
    let mut open: Option<ScriptOwner> = None;
    let mut any = false;
    for (index, sibling) in holder[scripts_field]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let tag = sibling.kind();
        if !tag.ends_with("_script") {
            continue;
        }
        let sibling_module = match module.object() {
            Some(object) => ScriptPath::ObjectScript {
                object,
                script: index,
            },
            None => ScriptPath::Module(index),
        };
        if sibling_module == *module {
            continue;
        }
        any = true;
        let (word, name, detail) = if tag == "shared_script" {
            let script = sibling.text_or("script", "?").to_owned();
            let values = sibling["values"]
                .as_object()
                .map(|v| {
                    v.iter()
                        .map(|(k, v)| format!("{k} {}", condition::summary(v)))
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .unwrap_or_default();
            ("shared", script, values)
        } else {
            let count = sibling["nodes"].as_object().map_or(0, |n| n.len());
            ("inline", format!("{count} nodes"), String::new())
        };
        let response = ui
            .horizontal(|ui| {
                widgets::badge(ui, "script");
                ui.label(theme::muted(word).monospace().size(13.0));
                ui.label(theme::strong(&name).monospace().size(13.0));
                if !detail.is_empty() {
                    ui.label(theme::muted(detail).monospace().size(12.5));
                }
            })
            .response
            .interact(egui::Sense::click());
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            open = Some(if tag == "shared_script" {
                ScriptOwner::Shared {
                    key: sibling.text("script").to_owned(),
                }
            } else {
                ScriptOwner::Inline {
                    table: table.clone(),
                    key: key.clone(),
                    module: sibling_module,
                }
            });
        }
    }
    if !any {
        ui.horizontal(|ui| {
            widgets::badge(ui, "script");
            ui.label(theme::strong("none").monospace().size(13.0));
            let stands = match doc.kind {
                OwnerKind::Npc => "stands where it spawns",
                OwnerKind::Object => "does nothing else",
                _ => "nothing else",
            };
            ui.label(theme::muted(stands).monospace().size(12.5));
        });
    }
    if let Some(owner) = open {
        editor.open_tab(EditorTab::Script(owner));
    }
}

fn owner_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let ScriptOwner::Inline { table, key, module } = &doc.owner else {
        return;
    };
    let mut open = false;
    match module.object() {
        Some(index) => {
            let row = editor.row_value(table, key).unwrap_or(Value::Null);
            let module = &row["modules"][index];
            let name = module.text_or("name", "object").to_owned();
            let label = module["label"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_default();
            widgets::section(ui, "Owner");
            let response = ui
                .horizontal(|ui| {
                    widgets::badge(ui, "object");
                    ui.label(
                        theme::strong(format!("{table}/{key}"))
                            .monospace()
                            .size(13.0),
                    );
                    ui.label(theme::muted("·").size(13.0));
                    ui.label(theme::strong(&name).monospace().size(13.0));
                    if !label.is_empty() {
                        ui.label(theme::muted(label).monospace().size(12.5));
                    }
                })
                .response
                .interact(egui::Sense::click());
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                open = true;
            }
        }
        None if doc.kind == OwnerKind::Npc => {
            let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
                return;
            };
            let placed: Vec<String> = content
                .ids::<AreaDef>()
                .filter(|area| {
                    area::placed(&content, *area)
                        .iter()
                        .any(|place| matches!(&place.module, AreaModule::Resident(resident) if resident.npc.key(&content) == key))
                })
                .map(|area| area.key(&content).to_owned())
                .collect();
            widgets::section(
                ui,
                &format!(
                    "Owner · placed {}",
                    match placed.len() {
                        0 => "nowhere".to_owned(),
                        1 => "once".to_owned(),
                        n => format!("{n} times"),
                    }
                ),
            );
            let response = ui
                .horizontal(|ui| {
                    widgets::badge(ui, "npc");
                    ui.label(theme::strong(format!("npc/{key}")).monospace().size(13.0));
                    match placed.as_slice() {
                        [] => ui.label(theme::muted("no resident yet").monospace().size(12.5)),
                        [one] => ui.label(
                            theme::muted(format!("resident of area/{one}"))
                                .monospace()
                                .size(12.5),
                        ),
                        many => ui.label(
                            theme::muted(format!("resident of {} areas", many.len()))
                                .monospace()
                                .size(12.5),
                        ),
                    };
                })
                .response
                .interact(egui::Sense::click());
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                open = true;
            }
        }
        None => {}
    }
    if open {
        editor.select(RowSelection::row(table, key));
        editor.open_tab(EditorTab::for_row(table, key));
    }
}

fn variables_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let key = doc.owner.key().to_owned();
    let variables: Vec<(String, Value)> = doc.script["variables"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if variables.is_empty() {
        return;
    }
    widgets::section_count(ui, "Variables", variables.len());
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of("script");
    let variable_schema = root
        .pointer("/properties/variables/additionalProperties")
        .cloned()
        .unwrap_or(Value::Null);
    let users = editor
        .project
        .as_ref()
        .map(|project| {
            project
                .usages
                .of_row("script", &key)
                .iter()
                .filter(|u| ScriptRef::parse(&u.pointer).node.is_none())
                .count()
        })
        .unwrap_or(0);
    let mut script = doc.script.clone();
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        for (name, variable) in &variables {
            let filled: Vec<String> = doc
                .nodes()
                .iter()
                .filter(|(_, node)| {
                    let mut names = Vec::new();
                    holes(node, &mut names);
                    names.contains(name)
                })
                .map(|(node_name, _)| node_name.clone())
                .collect();
            widgets::card_frame(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, name, "", |ui| {
                    if ui.button("Remove the variable").clicked() {
                        script["variables"].remove_field(name);
                        changed = FormChange::at("/variables");
                        ui.close();
                    }
                });
                let path = format!("/variables/{name}");
                let mut draft = variable.clone();
                if forms::object_without(
                    ui,
                    &mut cx,
                    &variable_schema,
                    &mut draft,
                    &path,
                    &["default"],
                )
                .is_some()
                {
                    script["variables"][name] = draft.clone();
                    changed = FormChange::at(&path);
                }
                widgets::kv(ui, "default", |ui| match draft.get("default") {
                    Some(default) if !default.is_null() => {
                        let mut value = default.clone();
                        let schema = variable_schema
                            .pointer("/properties/default")
                            .cloned()
                            .unwrap_or(Value::Null);
                        if forms::value(
                            ui,
                            &mut cx,
                            &schema,
                            &mut value,
                            &format!("{path}/default"),
                        )
                        .is_some()
                        {
                            script["variables"][name]["default"] = value;
                            changed = FormChange::at(&path);
                        }
                        if ui
                            .add(
                                egui::Label::new(theme::faint("✕").size(11.0))
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Drop the default")
                            .clicked()
                        {
                            script["variables"][name]
                                .as_object_mut()
                                .map(|m| m.remove("default"));
                            changed = FormChange::at(&path);
                        }
                    }
                    _ => {
                        ui.label(theme::muted("none").size(13.0));
                        if ui.button("+ Add default").clicked() {
                            script["variables"][name]["default"] = Value::Null;
                            changed = FormChange::at(&path);
                        }
                    }
                });
                widgets::kv_text(
                    ui,
                    "holes",
                    match filled.as_slice() {
                        [] => "none yet".to_owned(),
                        [one] => format!("1 · in {one}"),
                        many => format!("{} · in {}", many.len(), many.join(", ")),
                    },
                );
                widgets::kv_text(
                    ui,
                    "values",
                    match users {
                        0 => "no owner names this script".to_owned(),
                        1 => "1 owner gives its own".to_owned(),
                        n => format!("{n} {} give their own", doc.kind.name().to_owned() + "s"),
                    },
                );
            });
        }
    }
    for action in actions {
        act(editor, action, doc);
    }
    if changed.is_some() {
        write(
            editor,
            doc,
            script,
            format!("Edit the variables of script/{key}"),
        );
    }
}

struct Caller {
    owner: String,
    kind: &'static str,
    node: String,
}

fn run_by_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let key = doc.owner.key().to_owned();
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let callers: Vec<Caller> = project
        .usages
        .of_row("script", &key)
        .iter()
        .filter_map(|usage| {
            let (table, row_key) = row_of(&usage.file)?;
            let at = ScriptRef::parse(&usage.pointer);
            Some(Caller {
                owner: format!("{table}/{row_key}"),
                kind: if at.module.is_some() {
                    "inline"
                } else {
                    "shared"
                },
                node: at.node?,
            })
        })
        .collect();
    if !callers.is_empty() {
        widgets::section_count(ui, "Run by", callers.len());
        let mut open: Option<String> = None;
        for caller in &callers {
            let response = ui
                .horizontal(|ui| {
                    widgets::badge(ui, "script");
                    ui.label(theme::strong(&caller.owner).monospace().size(13.0));
                    ui.label(
                        theme::muted(format!("· {}", caller.kind))
                            .monospace()
                            .size(12.5),
                    );
                    ui.label(
                        theme::muted(format!("from run · {}", caller.node))
                            .monospace()
                            .size(12.5),
                    );
                })
                .response
                .interact(egui::Sense::click());
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                open = Some(caller.owner.clone());
            }
        }
        if let Some((table, row_key)) = open.as_deref().and_then(|owner| owner.split_once('/')) {
            editor.select(RowSelection::row(table, row_key));
            editor.open_tab(EditorTab::for_row(table, row_key));
        }
    }
    let speakers: BTreeSet<String> = doc
        .nodes()
        .values()
        .flat_map(|node| node["lines"].as_array().cloned().unwrap_or_default())
        .filter_map(|line| LineSpeaker::of(&line["by"]))
        .filter_map(|speaker| speaker.npc().map(str::to_owned))
        .collect();
    if !speakers.is_empty() {
        widgets::section(ui, "Speaks for");
        for npc in &speakers {
            ui.horizontal(|ui| {
                widgets::badge(ui, "npc");
                ui.label(theme::strong(format!("npc/{npc}")).monospace().size(13.0));
                let through = callers
                    .iter()
                    .find(|caller| caller.owner == format!("npc/{npc}"))
                    .map(|caller| format!("through its {} script", caller.kind));
                ui.label(
                    theme::muted(through.unwrap_or_else(|| "in its lines".to_owned()))
                        .monospace()
                        .size(12.5),
                );
            });
        }
    }
}

fn action_buttons(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    ui.add_space(8.0);
    let outline = editor
        .views
        .script
        .view(&doc.owner)
        .is_some_and(|view| view.outline);
    let mut fire = false;
    let mut toggle_outline = false;
    let mut make = false;
    ui.horizontal(|ui| {
        if widgets::keyed_button(ui, "▶ Run", "F8", true).clicked() {
            fire = true;
        }
        if widgets::keyed_button(ui, if outline { "Canvas" } else { "Outline" }, "O", true)
            .clicked()
        {
            toggle_outline = true;
        }
        if ui
            .button(if doc.is_shared() {
                "Make inline…"
            } else {
                "Make shared…"
            })
            .clicked()
        {
            make = true;
        }
    });
    if fire {
        editor.panels.bottom = BottomTab::Run;
        editor.views.script.fire(&doc.owner);
    }
    if toggle_outline {
        editor
            .views
            .script
            .view_mut(&doc.owner, &editor.user, editor.project.as_ref())
            .toggle_outline();
    }
    if make {
        if doc.is_shared() {
            make_inline_dialog(editor, doc);
        } else {
            make_shared_dialog(editor, doc);
        }
    }
}

fn node_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, name: &str) {
    let Some(node) = doc.nodes().get(name).cloned() else {
        return;
    };
    let kind = node.kind().to_owned();
    breadcrumb(ui, &format!("{} › {name}", owner_short(editor, doc)));
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of("script");
    let path = format!("{}/nodes/{name}", doc.owner.pointer());
    let (outs, ins, blurb) = {
        let catalogue = catalogue_of(editor.project.as_ref());
        let ins: Vec<(String, String)> = nodes::arrows_into(doc.nodes(), name, catalogue)
            .into_iter()
            .map(|(from, field)| {
                let label = nodes::port_label(doc.nodes(), catalogue, &from, &field);
                (from, label)
            })
            .collect();
        let entry = catalogue.entry(&kind);
        (
            nodes::ports(&node, catalogue),
            ins,
            entry.map_or("", |e| e.blurb),
        )
    };
    let mut draft = node.clone();
    let mut actions = Vec::new();
    let mut label = format!("Edit {name} in {}", doc.owner.title());
    let mut rename = false;
    let mut delete = false;
    let mut duplicate = false;
    let title = super::graph::card_title(name, &node);
    let changed = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        widgets::card_frame(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, &title, "", |ui| {
                    if ui.button("Rename…  F2").clicked() {
                        rename = true;
                        ui.close();
                    }
                    if ui.button("Duplicate  Ctrl D").clicked() {
                        duplicate = true;
                        ui.close();
                    }
                    if ui.button("Delete  Del").clicked() {
                        delete = true;
                        ui.close();
                    }
                });
                let mut changed = FormChange::default();
                match nodes::FlowKind::of(&draft) {
                    Some(nodes::FlowKind::On) => {
                        changed = on_form(ui, &mut cx, root, &mut draft, &path);
                    }
                    Some(nodes::FlowKind::Dialogue) => {}
                    _ => {
                        if let Some((schema, hidden)) = nodes::form_fields(root, &draft) {
                            let hidden: Vec<&str> = hidden.iter().map(String::as_str).collect();
                            changed = forms::object_without(
                                ui, &mut cx, schema, &mut draft, &path, &hidden,
                            );
                        }
                    }
                }
                if !nodes::is_dialogue(&node) {
                    arrows_rows(ui, &mut cx, &outs, &kind);
                }
                changed
            })
            .inner
    };
    let mut go_to: Option<String> = None;
    for action in actions {
        if let FormAction::OpenRow { table, key } = &action
            && table == "node"
        {
            go_to = Some(key.clone());
            continue;
        }
        act(editor, action, doc);
    }
    if nodes::is_dialogue(&node) {
        let mut draft_dialogue = draft.clone();
        let mut dialogue_actions = Vec::new();
        let dialogue_changed = {
            let mut cx = doc.form_cx(editor, root, &mut dialogue_actions);
            dialogue_form(
                ui,
                &mut cx,
                root,
                &mut draft_dialogue,
                &path,
                doc,
                &mut label,
            )
        };
        for action in dialogue_actions {
            if let FormAction::OpenRow { table, key } = &action
                && table == "node"
            {
                go_to = Some(key.clone());
                continue;
            }
            act(editor, action, doc);
        }
        if dialogue_changed.is_some() {
            let mut script = doc.script.clone();
            script["nodes"][name] = draft_dialogue;
            write(editor, doc, script, label.clone());
            return;
        }
    } else {
        node_extras(
            editor,
            ui,
            doc,
            Shown {
                name,
                node: &node,
                kind: &kind,
                ins: &ins,
                blurb,
            },
        );
    }
    if let Some(callout) = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(&doc.owner) && callout.node == name)
    {
        ui.label(theme::strong(&callout.rule).size(13.0).color(theme::DANGER));
        ui.label(theme::muted(&callout.text).size(12.5));
    }
    if let Some(target) = go_to {
        select_nodes(editor, doc, &[target]);
        return;
    }
    if rename {
        editor
            .dialogs
            .push(ModalDialog::RenameNode(RenameNodeDialog {
                owner: doc.owner.clone(),
                old: name.to_owned(),
                draft: name.to_owned(),
            }));
    }
    if duplicate {
        super::act(
            editor,
            doc,
            super::graph::ScriptGraphAction::Duplicate(name.to_owned()),
        );
    }
    if delete {
        super::delete_nodes(editor, doc, vec![name.to_owned()]);
        return;
    }
    if changed.is_some() {
        let mut script = doc.script.clone();
        script["nodes"][name] = draft;
        write(editor, doc, script, label);
    }
}

fn arrows_rows(ui: &mut Ui, cx: &mut FormCx, outs: &[nodes::NodePort], kind: &str) {
    let mut go: Option<String> = None;
    for port in outs {
        widgets::kv(ui, &port.label, |ui| match &port.target {
            Some(target) => {
                ui.label(theme::muted("→").size(13.0));
                if ui
                    .add(
                        egui::Label::new(theme::mono(target).size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    go = Some(target.clone());
                }
            }
            None => {
                let meaning = nodes::arrow_meaning(kind, &port.label);
                ui.add(egui::Label::new(theme::muted(meaning).size(13.0)).wrap());
            }
        });
    }
    if let Some(target) = go {
        cx.actions.push(FormAction::OpenRow {
            table: "node".to_owned(),
            key: target,
        });
    }
}

struct Shown<'a> {
    name: &'a str,
    node: &'a Value,
    kind: &'a str,
    ins: &'a [(String, String)],
    blurb: &'a str,
}

fn node_extras(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, shown: Shown) {
    let Shown {
        name,
        node,
        kind,
        ins,
        blurb,
    } = shown;
    match kind {
        "remember" | "forget" => {
            let memory = node["memory"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| node["memories"][0].as_str().map(str::to_owned));
            if let Some(memory) = memory {
                memory_row_card(editor, ui, doc, &memory);
            }
            keeps_cards(ui, doc);
        }
        "repeat" => bounds_card(editor, ui, doc, name, node),
        _ => {
            if !blurb.is_empty() {
                widgets::hint(ui, blurb);
            }
        }
    }
    if ins.len() > 1 {
        widgets::section(ui, "Arrows in");
        for (from, label) in ins {
            ui.horizontal(|ui| {
                ui.label(theme::mono(from).size(12.5));
                ui.label(theme::muted(label).size(12.5));
            });
        }
    }
}

fn memory_row_card(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, memory: &str) {
    let Some(mut row) = editor.row_value("memory", memory) else {
        return;
    };
    let shapes = editor.shapes.clone();
    let schema = shapes.shape_of("memory");
    let mut actions = Vec::new();
    let changed = {
        let mut cx = doc.form_cx(editor, schema, &mut actions);
        widgets::card_frame(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, &format!("memory/{memory}"), "row", |ui| {
                    if ui.button("Open the row").clicked() {
                        cx.actions.push(FormAction::OpenRow {
                            table: "memory".to_owned(),
                            key: memory.to_owned(),
                        });
                        ui.close();
                    }
                });
                forms::object_without(ui, &mut cx, schema, &mut row, "", &["modules"])
            })
            .inner
    };
    for action in actions {
        act(editor, action, doc);
    }
    if changed.is_some() {
        let mut edit = ContentEdit::new(format!("Edit memory/{memory}"));
        edit.write_json(row_file("memory", memory), &row);
        editor.apply(edit);
    }
}

fn keeps_cards(ui: &mut Ui, doc: &ScriptDoc) {
    let owner = doc.owner.key().to_owned();
    for (title, field, sentence) in [
        ("state", "state", format!("{owner}'s, shared by its flows")),
        ("locals", "locals", "values that last one flow".to_owned()),
    ] {
        widgets::card_frame(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::card_title(ui, title);
            let names = names_of(&doc.script[field]);
            ui.label(theme::muted(format!("{names} · {sentence}")).size(13.0));
        });
    }
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, "variables");
        let text = if doc.is_shared() {
            format!(
                "{} · each owner's values",
                names_of(&doc.script["variables"])
            )
        } else {
            "only in a shared script: each owner's values".to_owned()
        };
        ui.label(theme::muted(text).size(13.0));
    });
}

fn bounds_card(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, name: &str, node: &Value) {
    let catalogue = catalogue_of(editor.project.as_ref());
    let at_most = node["at_most"].as_f64().unwrap_or(1.0);
    let mut body: Vec<String> = Vec::new();
    let mut queue: Vec<String> = node["each"]
        .as_str()
        .map(|s| vec![s.to_owned()])
        .unwrap_or_default();
    while let Some(next) = queue.pop() {
        if next == name || body.contains(&next) {
            continue;
        }
        body.push(next.clone());
        if let Some(inner) = doc.nodes().get(&next) {
            for port in nodes::ports(inner, catalogue) {
                if let Some(target) = port.target {
                    queue.push(target);
                }
            }
        }
    }
    let waits: Vec<f64> = body
        .iter()
        .filter_map(|n| doc.nodes().get(n))
        .filter(|n| n.kind() == "wait")
        .map(|n| {
            n["at_most"]
                .as_f64()
                .or(n["seconds"].as_f64())
                .unwrap_or(0.0)
        })
        .collect();
    let pauses = body
        .iter()
        .filter_map(|n| doc.nodes().get(n))
        .any(|n| catalogue.entry(n.kind()).is_some_and(|e| e.pauses));
    let steps = if pauses {
        body.len() + 1
    } else {
        (body.len() + 1) * at_most as usize
    };
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, "bounds", "compiler", |_| {});
        widgets::kv_text(ui, "steps", format!("at most {steps} between two waits"));
        widgets::kv_text(
            ui,
            "waits",
            match waits.as_slice() {
                [] => "none in a pass".to_owned(),
                [one] => format!("at most {} · {} s in all", at_most as i64, one * at_most),
                many => format!(
                    "at most {} · {} s in all",
                    at_most as i64 * many.len() as i64,
                    many.iter().sum::<f64>() * at_most
                ),
            },
        );
    });
}

fn on_form(ui: &mut Ui, cx: &mut FormCx, root: &Value, node: &mut Value, path: &str) -> FormChange {
    let mut changed = FormChange::default();
    let events = nodes::event_variants(root);
    let current = nodes::event_of(node).unwrap_or("").to_owned();
    let mut picked = current.clone();
    widgets::kv(ui, "event", |ui| {
        widgets::combo_box(ui.id().with(path).with("event"))
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for variant in &events {
                    ui.selectable_value(&mut picked, variant.tag.clone(), &variant.tag);
                }
            });
    });
    if picked != current
        && let Some(fresh) = nodes::template_event(root, &picked)
    {
        let next = node.get("next").cloned();
        *node = fresh;
        if let Some(next) = next {
            node["next"] = next;
        }
        changed = FormChange::at(path);
    }
    if let Some(schema) = nodes::event_variant(root, nodes::event_of(node).unwrap_or("")) {
        changed |= forms::object_without(ui, cx, schema, node, path, &["event"]);
    }
    changed
}

fn dialogue_form(
    ui: &mut Ui,
    cx: &mut FormCx,
    root: &Value,
    node: &mut Value,
    path: &str,
    doc: &ScriptDoc,
    label: &mut String,
) -> FormChange {
    let mut changed = FormChange::default();
    let Some(schema) = nodes::variant_schema(root, "dialogue") else {
        return changed;
    };
    let line_schema = schema
        .pointer("/properties/lines/items")
        .cloned()
        .unwrap_or(Value::Null);
    let by_schema = line_schema
        .pointer("/properties/by")
        .cloned()
        .unwrap_or(Value::Null);
    let choice_schema = schema
        .pointer("/properties/choices/items")
        .cloned()
        .unwrap_or(Value::Null);
    widgets::section(ui, "Lines");
    let mut remove_line = None;
    let mut swap_line = None;
    let count = node["lines"].as_array().map_or(0, Vec::len);
    for index in 0..count {
        let line_path = format!("{path}/lines/{index}");
        let line = &mut node["lines"][index];
        widgets::card_frame(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::card_header(ui, &format!("line {}", index + 1), "", |ui| {
                if index > 0 && ui.button("Move up").clicked() {
                    swap_line = Some((index, index - 1));
                    ui.close();
                }
                if index + 1 < count && ui.button("Move down").clicked() {
                    swap_line = Some((index, index + 1));
                    ui.close();
                }
                if count > 1 && ui.button("Remove the line").clicked() {
                    remove_line = Some(index);
                    ui.close();
                }
            });
            changed |= speaker_rows(
                ui,
                cx,
                &by_schema,
                &mut line["by"],
                &format!("{line_path}/by"),
                doc,
            );
            changed |= spans::line_field(ui, cx, &mut line["text"], &format!("{line_path}/text"));
        });
    }
    if let Some(index) = remove_line
        && let Some(lines) = node["lines"].as_array_mut()
        && lines.len() > 1
    {
        lines.remove(index);
        changed = FormChange::at(path);
        *label = format!("Remove a line in {}", doc.owner.title());
    }
    if let Some((a, b)) = swap_line
        && let Some(lines) = node["lines"].as_array_mut()
    {
        lines.swap(a, b);
        changed = FormChange::at(path);
    }
    if ui
        .button("+ line")
        .on_hover_text("Enter on the card")
        .clicked()
    {
        if let Some(lines) = node["lines"].as_array_mut() {
            lines.push(doc.default_speaker().blank_line());
        }
        changed = FormChange::at(path);
        *label = format!("Add a line in {}", doc.owner.title());
    }
    widgets::section(ui, "Choices");
    let mut go: Option<String> = None;
    let count = node["choices"].as_array().map_or(0, Vec::len);
    let mut menu = ChoiceMenu::default();
    for index in 0..count {
        let at = ChoiceAt {
            schema: &choice_schema,
            path,
            index,
            count,
            doc,
        };
        let outcome = choice_card(ui, cx, &mut node["choices"][index], &at);
        changed |= outcome.changed;
        menu = menu.or(outcome.menu);
        go = go.or(outcome.go);
    }
    if let Some(index) = menu.remove
        && let Some(choices) = node["choices"].as_array_mut()
    {
        choices.remove(index);
        changed = FormChange::at(path);
        *label = format!("Remove a choice in {}", doc.owner.title());
    }
    if let Some((a, b)) = menu.swap
        && let Some(choices) = node["choices"].as_array_mut()
    {
        choices.swap(a, b);
        changed = FormChange::at(path);
    }
    if ui
        .button("+ choice")
        .on_hover_text("Ctrl Enter on the card")
        .clicked()
    {
        super::add_choice(node);
        changed = FormChange::at(path);
        *label = format!("Add a choice in {}", doc.owner.title());
    }
    if let Some(target) = go {
        cx.actions.push(FormAction::OpenRow {
            table: "node".to_owned(),
            key: target,
        });
    }
    changed
}

#[derive(Default)]
struct ChoiceMenu {
    remove: Option<usize>,
    swap: Option<(usize, usize)>,
}

impl ChoiceMenu {
    fn or(self, other: ChoiceMenu) -> ChoiceMenu {
        ChoiceMenu {
            remove: self.remove.or(other.remove),
            swap: self.swap.or(other.swap),
        }
    }
}

struct ChoiceAt<'a> {
    schema: &'a Value,
    path: &'a str,
    index: usize,
    count: usize,
    doc: &'a ScriptDoc,
}

#[derive(Default)]
struct ChoiceOutcome {
    changed: FormChange,
    menu: ChoiceMenu,
    go: Option<String>,
}

fn choice_card(ui: &mut Ui, cx: &mut FormCx, choice: &mut Value, at: &ChoiceAt) -> ChoiceOutcome {
    let ChoiceAt {
        schema: choice_schema,
        path,
        index,
        count,
        doc,
    } = *at;
    let choice_path = &format!("{path}/choices/{index}");
    let mut changed = FormChange::default();
    let mut menu = ChoiceMenu::default();
    let mut go: Option<String> = None;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let id = ui.id().with(choice_path).with("label");
            let edit = spans::line(
                ui,
                &choice["label"],
                id,
                egui::FontId::new(13.5, theme::medium()),
                "the choice",
            );
            if let Some(spans) = edit.spans {
                choice["label"] = spans;
                changed = FormChange::at(choice_path);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(theme::faint("…"), |ui| {
                    if index > 0 && ui.button("Move up").clicked() {
                        menu.swap = Some((index, index - 1));
                        ui.close();
                    }
                    if index + 1 < count && ui.button("Move down").clicked() {
                        menu.swap = Some((index, index + 1));
                        ui.close();
                    }
                    if ui.button("Remove the choice").clicked() {
                        menu.remove = Some(index);
                        ui.close();
                    }
                });
            });
        });
        widgets::kv(ui, "condition", |ui| match choice.get("requires") {
            Some(requires) if !requires.is_null() => {
                let schema = choice_schema
                    .pointer("/properties/requires")
                    .cloned()
                    .unwrap_or(serde_json::json!({ "x-condition": true }));
                let mut draft = requires.clone();
                ui.vertical(|ui| {
                    if condition::edit(
                        ui,
                        cx,
                        &schema,
                        &mut draft,
                        &format!("{choice_path}/requires"),
                    )
                    .is_some()
                    {
                        choice["requires"] = draft;
                        changed = FormChange::at(choice_path);
                    }
                    if ui
                        .add(
                            egui::Label::new(theme::faint("✕ no condition").size(11.5))
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        choice.remove_field("requires");
                        changed = FormChange::at(choice_path);
                    }
                });
            }
            _ => {
                if ui
                    .add(
                        egui::Label::new(theme::muted("none").size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Add a condition")
                    .clicked()
                {
                    choice["requires"] = Value::Bool(true);
                    changed = FormChange::at(choice_path);
                }
            }
        });
        widgets::kv(ui, "reveal", |ui| {
            let mut reveal: Vec<String> = choice["reveal"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let mut touched = false;
            ui.horizontal_wrapped(|ui| {
                for option in ["locked", "needs", "costs", "gains"] {
                    let on = reveal.iter().any(|r| r == option);
                    if ui
                        .selectable_label(on, theme::mono(option).size(12.0))
                        .clicked()
                    {
                        if on {
                            reveal.retain(|r| r != option);
                        } else {
                            reveal.push(option.to_owned());
                        }
                        touched = true;
                    }
                }
                if reveal.is_empty() {
                    ui.label(theme::muted("nothing").size(12.5));
                }
            });
            if touched {
                if reveal.is_empty() {
                    choice.remove_field("reveal");
                } else {
                    choice["reveal"] =
                        Value::Array(reveal.into_iter().map(Value::String).collect());
                }
                changed = FormChange::at(choice_path);
            }
        });
        widgets::kv(ui, "warning", |ui| {
            let mut warn = choice.get("warn").cloned().unwrap_or(Value::Null);
            if warn.is_null() {
                if ui
                    .add(
                        egui::Label::new(theme::muted("none").size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Warn before the choice")
                    .clicked()
                {
                    choice["warn"] = Value::String(String::new());
                    changed = FormChange::at(choice_path);
                }
            } else if forms::text(ui, cx, &mut warn, &format!("{choice_path}/warn")).is_some() {
                if warn.as_str().is_some_and(str::is_empty) {
                    choice.remove_field("warn");
                } else {
                    choice["warn"] = warn;
                }
                changed = FormChange::at(choice_path);
            }
        });
        widgets::kv(ui, "arrow", |ui| match choice["to"].as_str() {
            Some(to) => {
                let target = to.to_owned();
                let what = doc
                    .nodes()
                    .get(&target)
                    .map(|n| super::graph::card_title(&target, n))
                    .unwrap_or_else(|| target.clone());
                if ui
                    .add(
                        egui::Label::new(theme::mono(format!("→ {what}")).size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    go = Some(target);
                }
            }
            None => {
                ui.label(theme::muted("none · ends the conversation").size(13.0));
            }
        });
    });
    ChoiceOutcome { changed, menu, go }
}

fn choice_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, name: &str, index: usize) {
    let Some(node) = doc.nodes().get(name).cloned() else {
        return;
    };
    let count = node["choices"].as_array().map_or(0, Vec::len);
    if index >= count {
        node_inspector(editor, ui, doc, name);
        return;
    }
    let label_text = spans::plain(&node["choices"][index]["label"]);
    breadcrumb(
        ui,
        &format!(
            "{} › {name} › {}",
            owner_short(editor, doc),
            widgets::clip(&label_text, 40)
        ),
    );
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of("script");
    let path = format!("{}/nodes/{name}", doc.owner.pointer());
    let choice_schema = nodes::variant_schema(root, "dialogue")
        .and_then(|schema| schema.pointer("/properties/choices/items").cloned())
        .unwrap_or(Value::Null);
    let mut draft = node.clone();
    let mut actions = Vec::new();
    let outcome = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        let at = ChoiceAt {
            schema: &choice_schema,
            path: &path,
            index,
            count,
            doc,
        };
        choice_card(ui, &mut cx, &mut draft["choices"][index], &at)
    };
    let ChoiceOutcome {
        mut changed,
        menu,
        go,
    } = outcome;
    for action in actions {
        act(editor, action, doc);
    }
    let mut label = format!("Edit a choice in {}", doc.owner.title());
    if let Some(removed) = menu.remove
        && let Some(choices) = draft["choices"].as_array_mut()
    {
        choices.remove(removed);
        changed = FormChange::at(&path);
        label = format!("Remove a choice in {}", doc.owner.title());
    }
    if let Some((a, b)) = menu.swap
        && let Some(choices) = draft["choices"].as_array_mut()
    {
        choices.swap(a, b);
        changed = FormChange::at(&path);
    }
    if let Some(target) = go {
        select_nodes(editor, doc, &[target]);
        return;
    }
    if changed.is_some() {
        let mut script = doc.script.clone();
        script["nodes"][name] = draft;
        write(editor, doc, script, label);
    }
}

fn speaker_rows(
    ui: &mut Ui,
    cx: &mut FormCx,
    schema: &Value,
    by: &mut Value,
    path: &str,
    doc: &ScriptDoc,
) -> FormChange {
    let mut changed = FormChange::default();
    let current = LineSpeaker::of(by).unwrap_or(LineSpeaker::Narrator);
    let mut picked = current.kind();
    widgets::kv(ui, "by", |ui| {
        widgets::combo_box(ui.id().with(path).with("speaker"))
            .selected_text(theme::mono(current.kind()).size(13.0))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for option in LineSpeaker::options(doc.kind) {
                    ui.selectable_value(&mut picked, option, *option);
                }
            });
    });
    if picked != current.kind() {
        let npc = cx.speaker.clone().unwrap_or_default();
        *by = LineSpeaker::named(picked, &npc).json();
        changed = FormChange::at(path);
    }
    let LineSpeaker::Npc { mut npc, face } = LineSpeaker::of(by).unwrap_or(LineSpeaker::Narrator)
    else {
        return changed;
    };
    widgets::kv(ui, "└ npc", |ui| {
        if picker::row(ui, cx, "npc", &mut npc, &format!("{path}/npc")) {
            by["npc"] = Value::String(npc.clone());
            changed = FormChange::at(path);
        }
    });
    cx.speaker = Some(npc.clone());
    let face_schema = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|alts| {
            alts.iter().find(|alt| {
                alt.pointer("/properties/type/const")
                    .and_then(Value::as_str)
                    == Some("npc")
            })
        })
        .and_then(|alt| alt.pointer("/properties/face"))
        .cloned()
        .unwrap_or(serde_json::json!({ "x-face": true, "type": ["string", "null"] }));
    let mut face_value = face.clone().map_or(Value::Null, Value::String);
    widgets::kv(ui, "└ face", |ui| {
        let speaker = LineSpeaker::Npc {
            npc: npc.clone(),
            face,
        };
        let ctx = ui.ctx().clone();
        if let Some(file) = nodes::bust_file(cx.content.as_ref(), &speaker)
            && let Some(thumb) = cx.state.thumbnails.image(&ctx, cx.files, &file)
        {
            ui.add(
                egui::Image::from_texture((thumb.texture.id(), egui::vec2(26.0, 26.0)))
                    .uv(thumbs::square_uv(thumb.size))
                    .corner_radius(4.0),
            );
        }
        if forms::value(
            ui,
            cx,
            &face_schema,
            &mut face_value,
            &format!("{path}/face"),
        )
        .is_some()
        {
            match face_value.as_str().filter(|face| !face.is_empty()) {
                Some(face) => by["face"] = Value::String(face.to_owned()),
                None => by.remove_field("face"),
            }
            changed = FormChange::at(path);
        }
    });
    changed
}

fn act(editor: &mut Editor, action: FormAction, doc: &ScriptDoc) {
    inspector::act(editor, action, doc.owner.table(), doc.owner.key());
}

#[derive(Clone, Debug)]
pub struct ScriptConstant {
    pub node: String,
    pub field: String,
    pub value: Value,
    pub ty: VariableType,
    pub table: Option<String>,
    pub name: String,
    pub ticked: bool,
}

pub struct MakeSharedDialog {
    pub owner: ScriptOwner,
    pub key: String,
    pub kind: String,
    pub constants: Vec<ScriptConstant>,
    pub error: Option<String>,
}

pub struct InlineScriptTarget {
    pub table: String,
    pub key: String,
    pub module: ScriptPath,
}

pub struct MakeInlineDialog {
    pub key: String,
    pub users: Vec<InlineScriptTarget>,
    pub picked: usize,
}

pub fn make_shared_dialog(editor: &mut Editor, doc: &ScriptDoc) {
    if doc.is_shared() {
        return;
    }
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of("script");
    let mut constants = Vec::new();
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    for (name, node) in doc.nodes() {
        let Some(schema) = nodes::variant_schema(root, node.kind()) else {
            continue;
        };
        for (field, field_schema) in schema::properties(schema) {
            if field == "type" {
                continue;
            }
            let Some(value) = node.get(field) else {
                continue;
            };
            let (ty, table) = match (schema::marker(field_schema), value) {
                (Some("x-table"), Value::String(_)) => (
                    VariableType::Row,
                    schema::table_of(field_schema).map(str::to_owned),
                ),
                (Some("x-place"), Value::Object(_)) => (VariableType::Place, None),
                (None, Value::Number(n)) if n.is_i64() => (VariableType::Integer, None),
                (None, Value::Number(_)) => (VariableType::Number, None),
                (None, Value::Bool(_)) => (VariableType::Boolean, None),
                (None, Value::String(_)) if schema::type_names(field_schema) == vec!["string"] => {
                    (VariableType::Text, None)
                }
                _ => continue,
            };
            let base = field.clone();
            let count = taken.entry(base.clone()).or_default();
            *count += 1;
            let var_name = if *count == 1 {
                base
            } else {
                format!("{name}_{field}")
            };
            constants.push(ScriptConstant {
                node: name.clone(),
                field: field.clone(),
                value: value.clone(),
                ty,
                table,
                name: var_name,
                ticked: false,
            });
        }
    }
    let files = editor.project.as_ref().map(|p| &p.files);
    let key = files.map_or_else(
        || doc.owner.key().to_owned(),
        |files| rows::free_key(files, "script", doc.owner.key()),
    );
    editor
        .dialogs
        .push(ModalDialog::MakeShared(MakeSharedDialog {
            owner: doc.owner.clone(),
            key,
            kind: doc.kind.name().to_owned(),
            constants,
            error: None,
        }));
}

pub fn make_shared_body(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: MakeSharedDialog,
) -> Option<MakeSharedDialog> {
    let mut outcome = None;
    let owner = dialog
        .owner
        .title()
        .split(" › ")
        .next()
        .unwrap_or("")
        .to_owned();
    let count = ScriptDoc::load(editor, &dialog.owner).map_or(0, |doc| doc.nodes().len());
    let problem = editor
        .project
        .as_ref()
        .and_then(|project| rows::key_problem(&project.files, "script", &dialog.key));
    let valid = problem
        .as_deref()
        .is_none_or(|problem| problem.contains("already exists"));
    let free = problem.is_none();
    dialogs::card(ctx, "make-shared", 620.0, |ui| {
        let pressed = dialogs::keys(ui);
        ui.label(RichText::new("Make shared").strong().size(18.0));
        ui.add_space(6.0);
        ui.label(
            theme::muted(format!(
                "The {count} nodes move into a new script row, which {owner} names in their place."
            ))
            .size(13.5),
        );
        ui.add_space(10.0);
        widgets::kv(ui, "key", |ui| {
            ui.scope(|ui| {
                ui.set_width(160.0);
                let response = forms::boxed_input(ui, None, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.key)
                            .frame(egui::Frame::NONE)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    )
                });
                widgets::default_focus(ui, &response);
            });
            let standing = if !valid {
                "letters, digits, _ and - only".to_owned()
            } else if free {
                format!("script/{} · free", dialog.key)
            } else {
                format!("script/{} · taken", dialog.key)
            };
            ui.label(theme::faint(standing).size(12.5).monospace());
        });
        widgets::kv(ui, "owner", |ui| {
            widgets::combo_box("make-shared-kind")
                .width(ui.available_width() - 8.0)
                .selected_text(theme::mono(&dialog.kind).size(13.0))
                .show_ui(ui, |ui| {
                    for kind in OwnerKind::VARIANTS {
                        ui.selectable_value(&mut dialog.kind, kind.name().to_owned(), kind.name());
                    }
                });
        });
        if !dialog.constants.is_empty() {
            ui.add_space(8.0);
            widgets::section(ui, "Turn constants into variables");
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for constant in &mut dialog.constants {
                        ui.horizontal(|ui| {
                            widgets::toggle(ui, &mut constant.ticked);
                            ui.add_space(8.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(300.0, 20.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(theme::mono(&constant.field).size(13.0));
                                    let shown = match &constant.value {
                                        Value::String(text) => text.clone(),
                                        other => condition::summary(other),
                                    };
                                    ui.label(theme::strong(widgets::clip(&shown, 32)).size(13.0));
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if constant.ticked {
                                        let what = match &constant.table {
                                            Some(table) => format!("variable {table} · row"),
                                            None => format!("variable {}", constant.ty.name()),
                                        };
                                        ui.add(
                                            egui::TextEdit::singleline(&mut constant.name)
                                                .desired_width(110.0)
                                                .font(egui::TextStyle::Monospace),
                                        );
                                        ui.label(theme::muted(what).size(12.5));
                                    } else {
                                        ui.label(theme::muted("stays a constant").size(12.5));
                                    }
                                },
                            );
                        });
                        widgets::rule(ui);
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(6.0);
            ui.label(theme::muted(format!("Variables start without defaults; {owner} gives today's values. Make inline is the way back.")).size(13.0));
        }
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled_ui(valid && free, |ui| {
                    widgets::primary(ui, "Make shared · one step")
                })
                .inner
                .clicked()
                || (pressed.enter && valid && free)
            {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() || pressed.escape {
                outcome = Some(false);
            }
        });
    });
    editor
        .status
        .left
        .push(format!("{owner} › inline script · make shared"));
    editor
        .status
        .right
        .push("Enter makes it · Esc cancels".to_owned());
    match outcome {
        Some(true) => match make_shared(editor, &dialog) {
            Ok(()) => None,
            Err(error) => {
                dialog.error = Some(error);
                Some(dialog)
            }
        },
        Some(false) => None,
        None => Some(dialog),
    }
}

fn make_shared(editor: &mut Editor, dialog: &MakeSharedDialog) -> Result<(), String> {
    let doc = ScriptDoc::load(editor, &dialog.owner).ok_or("the script is gone")?;
    let files = &editor.project.as_ref().ok_or("no project")?.files;
    if let Some(problem) = rows::key_problem(files, "script", &dialog.key) {
        return Err(problem);
    }
    let mut nodes_map = doc.nodes().clone();
    let mut variables = Map::new();
    let mut values = Map::new();
    for constant in dialog.constants.iter().filter(|c| c.ticked) {
        let name = constant.name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("{} is not a variable name", constant.name));
        }
        if let Some(node) = nodes_map.get_mut(&constant.node) {
            node[&constant.field] = serde_json::json!({ "var": name });
        }
        let mut declared = serde_json::json!({ "type": constant.ty.name() });
        if let Some(table) = &constant.table {
            declared["table"] = Value::String(table.clone());
        }
        variables.insert(name.to_owned(), declared);
        values.insert(name.to_owned(), constant.value.clone());
    }
    let mut script = serde_json::json!({ "owner": dialog.kind });
    for field in ["state", "locals"] {
        if let Some(value) = doc.script.get(field) {
            script[field] = value.clone();
        }
    }
    if !variables.is_empty() {
        script["variables"] = Value::Object(variables);
    }
    script["nodes"] = Value::Object(nodes_map);
    let mut module = serde_json::json!({ "type": "shared_script", "script": dialog.key });
    if !values.is_empty() {
        module["values"] = Value::Object(values);
    }
    let row = doc.row_with(module);
    let mut edit = ContentEdit::new(format!(
        "Make {} shared as script/{}",
        dialog.owner.title(),
        dialog.key
    ));
    edit.write_json(row_file("script", &dialog.key), &script);
    edit.write_json(doc.file.clone(), &row);
    if editor.apply(edit) {
        let shared = ScriptOwner::Shared {
            key: dialog.key.clone(),
        };
        editor.views.script.relocate(&dialog.owner, shared.clone());
        let from = EditorTab::Script(dialog.owner.clone());
        for (_, tab) in editor.dock.iter_all_tabs_mut() {
            if *tab == from {
                *tab = EditorTab::Script(shared.clone());
            }
        }
        for tab in editor.open_tabs.iter_mut() {
            if *tab == from {
                *tab = EditorTab::Script(shared.clone());
            }
        }
        editor.select(RowSelection::row("script", &dialog.key));
        Ok(())
    } else {
        Err("refused; see the message".to_owned())
    }
}

pub fn make_inline_dialog(editor: &mut Editor, doc: &ScriptDoc) {
    let ScriptOwner::Shared { key } = &doc.owner else {
        return;
    };
    let Some(project) = &editor.project else {
        return;
    };
    let mut users = Vec::new();
    for usage in project.usages.of_row("script", key) {
        let Some((table, row_key)) = row_of(&usage.file) else {
            continue;
        };
        let at = ScriptRef::parse(&usage.pointer);
        let Some(module) = at.module.filter(|_| table != "script" && at.node.is_none()) else {
            continue;
        };
        users.push(InlineScriptTarget {
            table: table.to_owned(),
            key: row_key.to_owned(),
            module,
        });
    }
    if users.is_empty() {
        editor.toast("No owner names this script, so there is nowhere to copy it inline");
        return;
    }
    editor
        .dialogs
        .push(ModalDialog::MakeInline(MakeInlineDialog {
            key: key.clone(),
            users,
            picked: 0,
        }));
}

pub fn make_inline_body(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: MakeInlineDialog,
) -> Option<MakeInlineDialog> {
    let mut outcome = None;
    let count = editor
        .row_value("script", &dialog.key)
        .map_or(0, |script| script["nodes"].as_object().map_or(0, Map::len));
    let variables = editor.row_value("script", &dialog.key).map_or(0, |script| {
        script["variables"].as_object().map_or(0, Map::len)
    });
    let names: Vec<String> = dialog
        .users
        .iter()
        .map(|target| {
            let index = target.module.module();
            editor
                .row_value(&target.table, &target.key)
                .and_then(|row| row["modules"][index]["name"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("module {index}"))
        })
        .collect();
    dialogs::card(ctx, "make-inline", 620.0, |ui| {
        let pressed = dialogs::keys(ui);
        ui.label(RichText::new("Make inline").strong().size(18.0));
        ui.add_space(6.0);
        ui.label(theme::muted(format!("The {count} nodes of script/{} are copied into one owner's row, its values written in. The script row stays.", dialog.key)).size(13.5));
        ui.add_space(10.0);
        widgets::section(ui, "Into which owner");
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (index, target) in dialog.users.iter().enumerate() {
                    let picked = dialog.picked == index;
                    let module = names.get(index).cloned().unwrap_or_default();
                    let response = egui::Frame::new()
                        .fill(if picked {
                            theme::ACCENT_SOFT
                        } else {
                            theme::PANEL
                        })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if picked { theme::ACCENT } else { theme::BORDER },
                        ))
                        .corner_radius(5.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                widgets::badge(ui, &target.table);
                                ui.label(
                                    theme::mono(format!("{}/{}", target.table, target.key))
                                        .size(13.0),
                                );
                                ui.label(theme::muted(module).size(12.5));
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if response.clicked() {
                        dialog.picked = index;
                    }
                }
            });
        ui.add_space(6.0);
        ui.label(
            theme::muted(if variables == 0 {
                "No variables: the nodes copy as they are.".to_owned()
            } else {
                format!(
                    "{variables} {} take the owner's values, or the defaults where it gives none.",
                    if variables == 1 {
                        "variable"
                    } else {
                        "variables"
                    }
                )
            })
            .size(13.0),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::primary(ui, "Make inline · one step").clicked() || pressed.enter {
                outcome = Some(true);
            }
            if ui.button("Cancel").clicked() || pressed.escape {
                outcome = Some(false);
            }
        });
    });
    editor
        .status
        .left
        .push(format!("script/{} · make inline", dialog.key));
    editor
        .status
        .right
        .push("Enter makes it · Esc cancels".to_owned());
    match outcome {
        Some(true) => {
            make_inline(editor, &dialog);
            None
        }
        Some(false) => None,
        None => Some(dialog),
    }
}

fn make_inline(editor: &mut Editor, dialog: &MakeInlineDialog) {
    let Some(target) = dialog.users.get(dialog.picked) else {
        return;
    };
    let (table, key, pointer) = (&target.table, &target.key, target.module.pointer());
    let Some(script) = editor.row_value("script", &dialog.key) else {
        return;
    };
    let Some(mut row) = editor.row_value(table, key) else {
        return;
    };
    let Some(module) = row.pointer(&pointer).cloned() else {
        return;
    };
    let mut values: Map<String, Value> = module["values"].as_object().cloned().unwrap_or_default();
    for (name, variable) in script["variables"].as_object().into_iter().flatten() {
        if !values.contains_key(name)
            && let Some(default) = variable.get("default")
        {
            values.insert(name.clone(), default.clone());
        }
    }
    let mut nodes_value = script["nodes"].clone();
    fill_holes(&mut nodes_value, &Value::Object(values), &BTreeMap::new());
    let mut inline = serde_json::json!({ "type": "inline_script", "nodes": nodes_value });
    for field in ["state", "locals"] {
        if let Some(value) = script.get(field)
            && !value.is_null()
        {
            inline[field] = value.clone();
        }
    }
    if let Some(slot) = row.pointer_mut(&pointer) {
        *slot = inline;
    }
    let mut edit = ContentEdit::new(format!(
        "Make script/{} inline in {table}/{key}",
        dialog.key
    ));
    edit.write_json(row_file(table, key), &row);
    if editor.apply(edit) {
        editor.select(RowSelection::row(table, key));
        editor.open_tab(EditorTab::Script(ScriptOwner::Inline {
            table: table.clone(),
            key: key.clone(),
            module: target.module,
        }));
    }
}
