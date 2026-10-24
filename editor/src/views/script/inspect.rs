use std::collections::{BTreeMap, BTreeSet};
use ui::controls;
use ui::egui;
use ui::form;
use ui::menu;
use ui::panel;
use ui::sheet;
use ui::theme;
use ui::words;

use engine::core::content::{ContentFile, FolderReader};
use engine::core::script::{EventDef, Expr, Fill, RunnerKindDef};
use engine::field;
use engine::systems::dialogue::ChoiceReveal;
use engine::systems::memory::MemoryDef;
use engine::systems::npc::NpcDef;
use engine::systems::script::{
    Choice, DialogueNode, Line, Node, RiftEffect, RiftEvent, ScriptDef, Speaker, VarValue, Variable,
};
use serde::Deserialize;
use serde_json::Value;
use strum::VariantArray;
use ui::egui::Ui;
use ui::glyph::{self, Mark};

use super::graph::ScriptGraphAction;
use super::nodes::{self, SpeakerKind};
use super::{RenameNodeDialog, ScriptDoc, ScriptRef, select_nodes, write};
use crate::app::{BottomTab, Editor, ModalDialog};
use crate::forms::{self, FieldHole, FormAction, FormChange, FormCx, FormHoles, HoleRequest};
use crate::keymap::EditorKey;
use crate::project::files::content_of;
use crate::project::{Project, RowSelection};
use crate::shell::browser::{self, RowMenu};
use crate::shell::{row_form, widgets};
use crate::tabs::shown_path;
use crate::{condition, picker, rows, schema, spans, thumbs};

pub fn show(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, selection: &RowSelection) {
    super::sync_with_content(editor);
    let names = selected_nodes(doc, selection);
    let choice: Option<(String, usize)> = selection
        .parts
        .iter()
        .find_map(|part| part.strip_prefix("choice:"))
        .and_then(|rest| rest.rsplit_once('#'))
        .and_then(|(name, index)| Some((name.to_owned(), index.parse().ok()?)));
    egui::ScrollArea::vertical()
        .id_salt(("script-inspector", doc.at.slug()))
        .show(ui, |ui| match names.as_slice() {
            [] => {
                if !super::run::inspector(editor, ui, doc) {
                    form::nothing_selected(ui, "Select a node to inspect it here.");
                }
            }
            [name] => {
                let name = name.clone();
                match &choice {
                    Some((node, index)) if *node == name => {
                        choice_inspector(editor, ui, doc, &name, *index)
                    }
                    _ => node_inspector(editor, ui, doc, &name),
                }
            }
            many => {
                breadcrumb(ui, &format!("{} › {} nodes", doc.at.title(), many.len()));
                panel::card(true).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    panel::title(ui, &format!("{} nodes", many.len()));
                    for name in many {
                        if let (Some(node), Some(project)) =
                            (doc.nodes().get(name), editor.project.as_ref())
                        {
                            let summary = nodes::summary(node, &doc.text(project));
                            ui.horizontal(|ui| {
                                ui.label(theme::mono(name).size(12.5));
                                ui.label(theme::muted(words::clip(&summary, 50)).size(12.5));
                            });
                        }
                    }
                    ui.add_space(6.0);
                    if controls::danger(ui, "Delete these nodes").clicked() {
                        super::delete_nodes(editor, doc, many.to_vec());
                    }
                });
            }
        });
}

fn breadcrumb(ui: &mut Ui, text: &str) {
    ui.label(theme::muted(text).monospace().size(13.0));
    ui.add_space(4.0);
}

fn selected_nodes(doc: &ScriptDoc, selection: &RowSelection) -> Vec<String> {
    selection
        .parts
        .iter()
        .filter_map(|part| part.strip_prefix("node:").map(str::to_owned))
        .filter(|name| doc.nodes().contains_key(name))
        .collect()
}

pub fn script_row(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    super::sync_with_content(editor);
    let key = doc.at.key.clone();
    if let Some(folder) = crate::shell::breadcrumb::show(ui, &key) {
        crate::shell::content_tree::reveal(editor, &folder);
    }
    ui.add_space(4.0);
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(ScriptDef::TABLE);
    let mut script = doc.edited();
    let mut actions = Vec::new();
    let what = "script";
    let mut picked = None;
    let changed = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        panel::card(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, shown_path(&key), what, |ui| {
                    let menu = RowMenu::of(&key);
                    picked = browser::row_menu(ui, &menu, false);
                });
                let mut changed = FormChange::default();
                form::kv(ui, "runs on", |ui| {
                    let path = field!(ScriptDef, runs_on);
                    changed |= forms::of_type(ui, &mut cx, &mut script.runs_on, path);
                });
                let variables = script.variables.len();
                form::kv_text(
                    ui,
                    "variables",
                    if variables == 0 {
                        "none".to_owned()
                    } else {
                        variables.to_string()
                    },
                );
                form::kv(ui, "state", |ui| {
                    let path = field!(ScriptDef, state);
                    changed |= forms::of_type(ui, &mut cx, &mut script.state, path);
                });
                form::kv(ui, "locals", |ui| {
                    let path = field!(ScriptDef, locals);
                    changed |= forms::of_type(ui, &mut cx, &mut script.locals, path);
                });
                let returns = doc
                    .nodes()
                    .values()
                    .any(|node| matches!(node, Node::Return { value: Some(_) }));
                form::kv_text(ui, "returns", if returns { "a value" } else { "nothing" });
                form::kv_text(ui, "nodes", nodes_line(doc));
                changed
            })
            .inner
    };
    for action in actions {
        act(editor, action, doc);
    }
    if changed.is_some() {
        write(editor, doc, script, format!("Edit {}", shown_path(&key)));
    }
    if let Some(action) = picked {
        browser::act(editor, ScriptDef::TABLE, &key, action);
    }
    events_section(editor, ui, doc);
    variables_section(editor, ui, doc);
    run_by_section(editor, ui, doc);
    action_buttons(editor, ui, doc);
}

pub fn names_of<V>(map: &BTreeMap<String, V>) -> String {
    let names: Vec<&str> = map.keys().map(String::as_str).collect();
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

fn events_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    sheet::section(ui, "Events");
    let mut go: Option<String> = None;
    let mut any = false;
    for (name, node) in doc.nodes() {
        if !nodes::is_event(node) {
            continue;
        }
        any = true;
        let Some(project) = editor.project.as_ref() else {
            return;
        };
        let event = nodes::event_name(node).unwrap_or("?").to_owned();
        let pieces = nodes::pieces(node, &doc.text(project));
        let detail = pieces
            .first()
            .map(|line| line.iter().map(|p| p.text.clone()).collect::<String>())
            .unwrap_or_default();
        let response = ui
            .horizontal(|ui| {
                glyph::show(ui, Mark::CaretRight, 12.0, theme::FAINT);
                ui.label(theme::strong(&event).monospace().size(13.0));
                ui.label(
                    theme::muted(detail.replace(" m", ""))
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
        form::hint(
            ui,
            &format!(
                "No event yet: {} opens the palette with the events this owner answers.",
                EditorKey::Palette.name()
            ),
        );
    }
    if let Some(name) = go {
        select_nodes(editor, doc, &[name]);
    }
}

fn variables_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let key = doc.at.key.clone();
    let variables: Vec<(String, Variable)> = doc
        .script
        .variables
        .iter()
        .map(|(name, variable)| (name.clone(), variable.clone()))
        .collect();
    if variables.is_empty() {
        return;
    }
    form::section_count(ui, "Variables", variables.len());
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(ScriptDef::TABLE);
    let users = editor
        .project
        .as_ref()
        .map(|project| {
            project
                .usages
                .of_content(&key)
                .iter()
                .filter(|u| ScriptRef::parse(&u.pointer).node.is_none())
                .count()
        })
        .unwrap_or(0);
    let mut script = doc.edited();
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        for (name, variable) in &variables {
            let filled: Vec<String> = doc
                .nodes()
                .iter()
                .filter(|(_, node)| nodes::node_vars(&doc.reader, node).contains(name))
                .map(|(node_name, _)| node_name.clone())
                .collect();
            panel::card(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, name, "", |ui| {
                    if ui.button("Remove the variable").clicked() {
                        script.variables.remove(name);
                        changed = FormChange::at(field!(ScriptDef, variables));
                        ui.close();
                    }
                });
                let path = format!("{}/{name}", field!(ScriptDef, variables));
                let mut draft = variable.clone();
                let mut edited = forms::of_type_without(
                    ui,
                    &mut cx,
                    &mut draft,
                    &path,
                    &[field!(Variable, default)],
                );
                if (draft.ty, &draft.table) != (variable.ty, &variable.table) {
                    draft.default = None;
                }
                form::kv(ui, "default", |ui| {
                    edited |= default_value(ui, &mut cx, &mut draft, &path);
                });
                if edited.is_some() && script.variables.contains_key(name) {
                    script.variables.insert(name.clone(), draft);
                    changed = FormChange::at(&path);
                }
                form::kv_text(
                    ui,
                    "holes",
                    match filled.as_slice() {
                        [] => "none yet".to_owned(),
                        [one] => format!("1 · in {one}"),
                        many => format!("{} · in {}", many.len(), many.join(", ")),
                    },
                );
                form::kv_text(
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
            format!("Edit the variables of {}", shown_path(&key)),
        );
    }
}

/// The value a variable takes when its owner gives none, edited as its type is.
fn default_value(ui: &mut Ui, cx: &mut FormCx, variable: &mut Variable, path: &str) -> FormChange {
    let reader = cx.project.reader();
    let shape = schema::variable_schema(variable.ty, variable.table.as_deref());
    let path = format!("{path}{}", field!(Variable, default));
    let mut changed = FormChange::default();
    match &variable.default {
        Some(default) => {
            let Ok(mut json) = reader.to_json(default) else {
                return changed;
            };
            if forms::value(ui, cx, &shape, &mut json, &path).is_some() {
                match reader.from_json::<VarValue>(&json) {
                    Ok(read) => {
                        variable.default = Some(read);
                        changed = FormChange::at(&path);
                    }
                    Err(error) => {
                        cx.state.errors.insert(path.clone(), error);
                    }
                }
            }
            if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                .on_hover_text("Drop the default")
                .clicked()
            {
                variable.default = None;
                changed = FormChange::at(&path);
            }
        }
        None => {
            ui.label(theme::muted("none").size(13.0));
            if ui.button("+ Add default").clicked() {
                let fresh = schema::template(&shape, &shape, cx.nearby());
                match reader.from_json::<VarValue>(&fresh) {
                    Ok(read) => {
                        variable.default = Some(read);
                        changed = FormChange::at(&path);
                    }
                    Err(error) => {
                        cx.state.errors.insert(path.clone(), error);
                    }
                }
            }
        }
    }
    changed
}

struct Caller {
    table: String,
    key: String,
    node: String,
}

fn run_by_section(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    let key = doc.at.key.clone();
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let callers: Vec<Caller> = project
        .usages
        .of_content(&key)
        .iter()
        .filter_map(|usage| {
            let (table, row_key) = content_of(&usage.file)?;
            let at = ScriptRef::parse(&usage.pointer);
            Some(Caller {
                table: table.to_owned(),
                key: row_key.to_owned(),
                node: at.node?,
            })
        })
        .collect();
    if !callers.is_empty() {
        form::section_count(ui, "Run by", callers.len());
        let mut open: Option<&Caller> = None;
        for caller in &callers {
            let response = ui
                .horizontal(|ui| {
                    widgets::badge(ui, "script");
                    ui.label(
                        theme::strong(shown_path(&caller.key))
                            .monospace()
                            .size(13.0),
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
                open = Some(caller);
            }
        }
        if let Some(caller) = open {
            editor.open_row(&caller.table, &caller.key);
        }
    }
    let speakers: BTreeSet<String> = doc
        .nodes()
        .values()
        .flat_map(|node| match node {
            Node::Dialogue(dialogue) => dialogue.lines.as_slice(),
            _ => &[],
        })
        .filter_map(|line| match line.by {
            Speaker::Npc(npc) => doc.reader.path_of(npc).map(str::to_owned),
            _ => None,
        })
        .collect();
    if !speakers.is_empty() {
        sheet::section(ui, "Speaks for");
        for npc in &speakers {
            ui.horizontal(|ui| {
                widgets::badge(ui, "npc");
                ui.label(theme::strong(shown_path(npc)).monospace().size(13.0));
                let through = callers
                    .iter()
                    .any(|caller| caller.table == NpcDef::TABLE && caller.key == *npc);
                ui.label(
                    theme::muted(if through {
                        "through its script"
                    } else {
                        "in its lines"
                    })
                    .monospace()
                    .size(12.5),
                );
            });
        }
    }
}

fn action_buttons(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc) {
    ui.add_space(8.0);
    if controls::keyed(ui, "Run", Some(&EditorKey::RunScript.name()), true).clicked() {
        editor.panels.bottom = BottomTab::Run;
        editor.views.script.fire(&doc.at);
    }
}

fn node_inspector(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, name: &str) {
    let Some(node) = doc.nodes().get(name).cloned() else {
        return;
    };
    breadcrumb(ui, &format!("{} › {name}", doc.at.title()));
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(ScriptDef::TABLE);
    let path = format!("/nodes/{name}");
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let catalogue = &project.catalogue;
    let ins: Vec<(String, String)> = nodes::arrows_into(doc.nodes(), name)
        .into_iter()
        .map(|(from, exit)| {
            let label = nodes::port_label(doc.nodes(), &from, exit);
            (from, label)
        })
        .collect();
    let outs = nodes::ports(&node);
    let unwired: Vec<String> = outs
        .iter()
        .map(|port| catalogue.unwired(&node, &port.label))
        .collect();
    let blurb = catalogue
        .entry_of(&node)
        .map_or(String::new(), |entry| entry.blurb.clone());
    let mut draft = node.clone();
    let mut actions = Vec::new();
    let mut hole_requests = Vec::new();
    let mut label = format!("Edit {name} in {}", doc.at.title());
    let mut rename = false;
    let mut delete = false;
    let mut duplicate = false;
    let title = super::graph::card_title(name, &node);
    let changed = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        panel::card(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, &title, "", |ui| {
                    if ui
                        .add(widgets::shortcut_button("Rename…", EditorKey::Rename))
                        .clicked()
                    {
                        rename = true;
                        ui.close();
                    }
                    if ui
                        .add(widgets::shortcut_button("Duplicate", EditorKey::Duplicate))
                        .clicked()
                    {
                        duplicate = true;
                        ui.close();
                    }
                    if ui
                        .add(widgets::shortcut_button("Delete", EditorKey::Delete))
                        .clicked()
                    {
                        delete = true;
                        ui.close();
                    }
                });
                let mut changed = FormChange::default();
                match &mut draft {
                    Node::On { event, .. } => {
                        changed = on_form(ui, &mut cx, &doc.reader, event, &path);
                    }
                    Node::Dialogue(_) => {}
                    other => {
                        if let Some((schema, hidden)) = nodes::form_fields(other) {
                            cx.holes = Some(node_holes(doc, schema, other, &path));
                            let hidden: Vec<&str> = hidden.iter().map(String::as_str).collect();
                            changed = forms::variant_of_type(
                                ui,
                                &mut cx,
                                nodes::node_root(),
                                schema,
                                other,
                                &path,
                                &hidden,
                            );
                        }
                    }
                }
                if !nodes::is_dialogue(&node) {
                    arrows_rows(ui, &mut cx, &outs, &unwired);
                }
                hole_requests = cx
                    .holes
                    .take()
                    .map(|holes| holes.requests)
                    .unwrap_or_default();
                changed
            })
            .inner
    };
    let mut go_to = act_but_go(editor, actions, doc);
    if let Node::Dialogue(dialogue) = &node {
        let mut draft_dialogue = dialogue.clone();
        let mut dialogue_actions = Vec::new();
        let dialogue_changed = {
            let mut cx = doc.form_cx(editor, root, &mut dialogue_actions);
            dialogue_form(ui, &mut cx, &mut draft_dialogue, &path, doc, &mut label)
        };
        go_to = act_but_go(editor, dialogue_actions, doc).or(go_to);
        if dialogue_changed.is_some() {
            let mut script = doc.edited();
            script
                .nodes
                .insert(name.to_owned(), Node::Dialogue(draft_dialogue));
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
                ins: &ins,
                blurb: &blurb,
            },
        );
    }
    if let Some(callout) = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(&doc.at) && callout.node == name)
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
                at: doc.at.clone(),
                old: name.to_owned(),
                draft: name.to_owned(),
            }));
    }
    if duplicate {
        super::act(
            editor,
            doc,
            ScriptGraphAction::Duplicate(vec![name.to_owned()]),
        );
    }
    if delete {
        super::delete_nodes(editor, doc, vec![name.to_owned()]);
        return;
    }
    if !hole_requests.is_empty() {
        let Some(project) = editor.project.as_ref() else {
            return;
        };
        let mut script = doc.edited();
        for request in hole_requests {
            fill_hole(doc, project, &mut script, &mut draft, &path, request);
        }
        script.nodes.insert(name.to_owned(), draft);
        write(editor, doc, script, label);
    } else if changed.is_some() {
        let mut script = doc.edited();
        script.nodes.insert(name.to_owned(), draft);
        write(editor, doc, script, label);
    }
}

/// The fields of `node` a variable of the script could fill, each filled by the variable it
/// names or open to one.
fn node_holes(doc: &ScriptDoc, schema: &Value, node: &Node, path: &str) -> FormHoles {
    let json = doc.reader.to_json(node).unwrap_or_default();
    let fields = schema::var_fields(nodes::node_root(), schema, &json)
        .into_iter()
        .map(|var| {
            let hole = match json.pointer(&var.field.pointer).and_then(var_name) {
                Some(name) => FieldHole::Filled(name),
                None => FieldHole::Open(var.field.kind),
            };
            (format!("{path}{}", var.field.pointer), hole)
        })
        .collect();
    FormHoles {
        fields,
        variables: doc
            .script
            .variables
            .iter()
            .map(|(name, variable)| (name.clone(), variable.ty))
            .collect(),
        requests: Vec::new(),
    }
}

/// Fills a field of `node` from a variable of `script`, a new one or one it has, or gives the
/// field back a value of its own.
fn fill_hole(
    doc: &ScriptDoc,
    project: &Project,
    script: &mut ScriptDef,
    node: &mut Node,
    path: &str,
    request: HoleRequest,
) {
    let Some((schema, _)) = nodes::form_fields(node) else {
        return;
    };
    let Ok(mut json) = doc.reader.to_json(&*node) else {
        return;
    };
    let fields = schema::var_fields(nodes::node_root(), schema, &json);
    let (field, filled) = match request {
        HoleRequest::Make { field, kind } => {
            let pointer = field.strip_prefix(path).unwrap_or(&field).to_owned();
            let table = fields
                .iter()
                .find(|var| var.field.pointer == pointer)
                .and_then(|var| var.field.table.clone());
            let base = pointer.rsplit('/').next().unwrap_or("value");
            let name = rows::free_name(base, &script.variables.keys().cloned().collect());
            script.variables.insert(
                name.clone(),
                Variable {
                    ty: kind,
                    table,
                    default: None,
                },
            );
            (pointer, VarFill::Var(name))
        }
        HoleRequest::Use { field, name } => (
            field.strip_prefix(path).unwrap_or(&field).to_owned(),
            VarFill::Var(name),
        ),
        HoleRequest::Fix { field, .. } => {
            let pointer = field.strip_prefix(path).unwrap_or(&field).to_owned();
            let Some(var) = fields.iter().find(|var| var.field.pointer == pointer) else {
                return;
            };
            let nearby = schema::Nearby::of(project, Some(&doc.file));
            let fixed = schema::template(nodes::node_root(), &var.fixed, nearby);
            (pointer, VarFill::Fixed(fixed))
        }
    };
    let Some(at) = json.pointer_mut(&field) else {
        return;
    };
    *at = match filled {
        VarFill::Var(name) => doc
            .reader
            .to_json(&Fill::<()>::Var { var: name })
            .unwrap_or_default(),
        VarFill::Fixed(fixed) => fixed,
    };
    if let Ok(read) = doc.reader.from_json(&json) {
        *node = read;
    }
}

enum VarFill {
    Var(String),
    Fixed(Value),
}

/// The variable a field's value names, when it names one rather than holding a value.
fn var_name(value: &Value) -> Option<String> {
    match Fill::<Value>::deserialize(value).ok()? {
        Fill::Var { var } => Some(var),
        Fill::Const(_) => None,
    }
}

fn arrows_rows(ui: &mut Ui, cx: &mut FormCx, outs: &[nodes::NodePort], unwired: &[String]) {
    let mut go: Option<String> = None;
    for (port, unwired) in outs.iter().zip(unwired) {
        form::kv(ui, &port.label, |ui| match &port.target {
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
                ui.add(egui::Label::new(theme::muted(unwired).size(13.0)).wrap());
            }
        });
    }
    if let Some(name) = go {
        cx.actions.push(FormAction::OpenNode { name });
    }
}

struct Shown<'a> {
    name: &'a str,
    node: &'a Node,
    ins: &'a [(String, String)],
    blurb: &'a str,
}

fn node_extras(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, shown: Shown) {
    let Shown {
        name,
        node,
        ins,
        blurb,
    } = shown;
    let memory = match node {
        Node::Effect(RiftEffect::Remember(remember)) => Some(Some(remember.memory)),
        Node::Effect(RiftEffect::Forget(forget)) => Some(forget.memories.first().copied()),
        _ => None,
    };
    match (node, memory) {
        (Node::Repeat { .. }, _) => bounds_card(ui, doc, name, node),
        (_, Some(memory)) => {
            if let Some(memory) = memory.and_then(|memory| doc.reader.path_of(memory)) {
                memory_row_card(editor, ui, doc, memory);
            }
            keeps_cards(ui, doc);
        }
        _ => {
            if !blurb.is_empty() {
                form::hint(ui, blurb);
            }
        }
    }
    if ins.len() > 1 {
        sheet::section(ui, "Arrows in");
        for (from, label) in ins {
            ui.horizontal(|ui| {
                ui.label(theme::mono(from).size(12.5));
                ui.label(theme::muted(label).size(12.5));
            });
        }
    }
}

fn memory_row_card(editor: &mut Editor, ui: &mut Ui, doc: &ScriptDoc, memory: &str) {
    let Some(Ok(read)) = editor.read::<MemoryDef>(memory) else {
        return;
    };
    let mut row = (*read).clone();
    let shapes = editor.shapes.clone();
    let schema = shapes.shape_of(MemoryDef::TABLE);
    let mut actions = Vec::new();
    let changed = {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        let mut cx = FormCx::new(project, schema, shapes, form, &mut actions)
            .editing(MemoryDef::TABLE, Some(memory));
        panel::card(false)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, shown_path(memory), "row", |ui| {
                    if ui.button("Open the row").clicked() {
                        cx.actions.push(FormAction::OpenRow {
                            table: MemoryDef::TABLE.to_owned(),
                            key: memory.to_owned(),
                        });
                        ui.close();
                    }
                });
                forms::of_type(ui, &mut cx, &mut row, "")
            })
            .inner
    };
    for action in actions {
        act(editor, action, doc);
    }
    if changed.is_some() {
        editor.write_content(&format!("Edit {}", shown_path(memory)), memory, &row);
    }
}

fn keeps_cards(ui: &mut Ui, doc: &ScriptDoc) {
    let runner = doc.kind.name();
    let script = &doc.script;
    for (title, names, sentence) in [
        (
            "state",
            names_of(&script.state),
            format!("its {runner}'s, shared by its flows"),
        ),
        (
            "locals",
            names_of(&script.locals),
            "values that last one flow".to_owned(),
        ),
    ] {
        panel::card(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            panel::title(ui, title);
            ui.label(theme::muted(format!("{names} · {sentence}")).size(13.0));
        });
    }
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "variables");
        let text = format!("{} · each runner's values", names_of(&script.variables));
        ui.label(theme::muted(text).size(13.0));
    });
}

fn bounds_card(ui: &mut Ui, doc: &ScriptDoc, name: &str, node: &Node) {
    let Node::Repeat { at_most, each, .. } = node else {
        return;
    };
    let at_most = f64::from(*at_most);
    let mut body: Vec<String> = Vec::new();
    let mut queue: Vec<String> = each.to().map(str::to_owned).into_iter().collect();
    while let Some(next) = queue.pop() {
        if next == name || body.contains(&next) {
            continue;
        }
        body.push(next.clone());
        if let Some(inner) = doc.nodes().get(&next) {
            queue.extend(
                inner
                    .exits()
                    .into_iter()
                    .filter_map(|(_, arrow)| arrow.to().map(str::to_owned)),
            );
        }
    }
    let inner: Vec<&Node> = body.iter().filter_map(|n| doc.nodes().get(n)).collect();
    let waits: Vec<f64> = inner
        .iter()
        .filter_map(|node| match node {
            Node::Wait { at_most, .. } => Some(f64::from(at_most.0)),
            _ => None,
        })
        .collect();
    let pauses = inner.iter().any(|node| node.pauses());
    let steps = if pauses {
        body.len() + 1
    } else {
        (body.len() + 1) * at_most as usize
    };
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "bounds", "compiler", |_| {});
        form::kv_text(ui, "steps", format!("at most {steps} between two waits"));
        form::kv_text(
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

fn on_form(
    ui: &mut Ui,
    cx: &mut FormCx,
    reader: &FolderReader,
    event: &mut RiftEvent,
    path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    let events = nodes::event_variants();
    let current: &str = event.kind().into();
    let mut picked = current.to_owned();
    form::kv(ui, "event", |ui| {
        controls::combo(ui.id().with(path).with("event"))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for variant in &events {
                    ui.selectable_value(&mut picked, variant.tag.clone(), &variant.tag);
                }
            });
    });
    if picked != current
        && let Some(Node::On { event: fresh, .. }) =
            nodes::fresh_event(reader, &picked, cx.nearby())
    {
        *event = fresh;
        changed = FormChange::at(path);
    }
    let shown: &str = event.kind().into();
    if let Some(variant) = events.iter().find(|variant| variant.tag == shown) {
        changed |= forms::variant_of_type(
            ui,
            cx,
            nodes::node_root(),
            variant.schema,
            event,
            path,
            &[variant.field.as_str()],
        );
    }
    changed
}

fn dialogue_form(
    ui: &mut Ui,
    cx: &mut FormCx,
    node: &mut DialogueNode,
    path: &str,
    doc: &ScriptDoc,
    label: &mut String,
) -> FormChange {
    let mut changed = FormChange::default();
    sheet::section(ui, "Lines");
    let mut remove_line = None;
    let mut swap_line = None;
    let count = node.lines.len();
    for (index, line) in node.lines.iter_mut().enumerate() {
        let line_path = format!("{path}/lines/{index}");
        panel::card(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            panel::header(ui, &format!("line {}", index + 1), "", |ui| {
                swap_line = menu::move_entries(ui, index, count).or(swap_line);
                if count > 1 && ui.button("Remove the line").clicked() {
                    remove_line = Some(index);
                    ui.close();
                }
            });
            changed |= speaker_rows(ui, cx, line, &format!("{line_path}/by"), doc);
            changed |= spans::line_field(ui, cx, &mut line.text, &format!("{line_path}/text"));
        });
    }
    if let Some(index) = remove_line
        && node.lines.len() > 1
    {
        node.lines.remove(index);
        changed = FormChange::at(path);
        *label = format!("Remove a line in {}", doc.at.title());
    }
    if let Some((a, b)) = swap_line {
        node.lines.swap(a, b);
        changed = FormChange::at(path);
    }
    if ui
        .button("+ line")
        .on_hover_text(format!("{} on the card", EditorKey::Confirm.name()))
        .clicked()
    {
        node.lines.push(nodes::blank_line(doc.default_speaker()));
        changed = FormChange::at(path);
        *label = format!("Add a line in {}", doc.at.title());
    }
    sheet::section(ui, "Choices");
    let mut go: Option<String> = None;
    let count = node.choices.len();
    let mut menu = ChoiceMenu::default();
    for (index, choice) in node.choices.iter_mut().enumerate() {
        let at = ChoiceAt {
            path,
            index,
            count,
            doc,
        };
        let outcome = choice_card(ui, cx, choice, &at);
        changed |= outcome.changed;
        menu = menu.or(outcome.menu);
        go = go.or(outcome.go);
    }
    if let Some(index) = menu.remove {
        node.choices.remove(index);
        changed = FormChange::at(path);
        *label = format!("Remove a choice in {}", doc.at.title());
    }
    if let Some((a, b)) = menu.swap {
        node.choices.swap(a, b);
        changed = FormChange::at(path);
    }
    if ui
        .button("+ choice")
        .on_hover_text(format!("{} on the card", EditorKey::AddChoice.name()))
        .clicked()
    {
        let mut dialogue = Node::Dialogue(node.clone());
        super::add_choice(&mut dialogue);
        if let Node::Dialogue(added) = dialogue {
            *node = added;
        }
        changed = FormChange::at(path);
        *label = format!("Add a choice in {}", doc.at.title());
    }
    if let Some(name) = go {
        cx.actions.push(FormAction::OpenNode { name });
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

fn choice_card(ui: &mut Ui, cx: &mut FormCx, choice: &mut Choice, at: &ChoiceAt) -> ChoiceOutcome {
    let ChoiceAt {
        path,
        index,
        count,
        doc,
    } = *at;
    let choice_path = &format!("{path}/choices/{index}");
    let mut changed = FormChange::default();
    let mut menu = ChoiceMenu::default();
    let mut go: Option<String> = None;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
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
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                let id = ui.id().with(choice_path).with("label");
                let edit = spans::line(
                    ui,
                    &choice.label,
                    id,
                    egui::FontId::new(13.5, theme::medium()),
                    "the choice",
                );
                if let Some(spans) = edit.spans {
                    choice.label = spans;
                    changed = FormChange::at(choice_path);
                }
            });
        });
        form::kv(ui, "condition", |ui| match &mut choice.requires {
            Some(requires) => {
                let mut dropped = false;
                ui.vertical(|ui| {
                    let requires_path = format!("{choice_path}/requires");
                    if condition::edit(ui, cx, requires, &requires_path).is_some() {
                        changed = FormChange::at(choice_path);
                    }
                    dropped = ui
                        .add(
                            egui::Label::new(theme::faint("remove the condition").size(11.5))
                                .sense(egui::Sense::click()),
                        )
                        .clicked();
                });
                if dropped {
                    choice.requires = None;
                    changed = FormChange::at(choice_path);
                }
            }
            None => {
                if ui
                    .add(
                        egui::Label::new(theme::muted("none").size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Add a condition")
                    .clicked()
                {
                    choice.requires = Some(Expr::Bool(true));
                    changed = FormChange::at(choice_path);
                }
            }
        });
        form::kv(ui, "reveal", |ui| {
            ui.horizontal_wrapped(|ui| {
                for &option in ChoiceReveal::VARIANTS {
                    let on = choice.reveal.contains(&option);
                    if ui
                        .selectable_label(on, theme::mono(<&str>::from(option)).size(12.0))
                        .clicked()
                    {
                        if on {
                            choice.reveal.retain(|held| *held != option);
                        } else {
                            choice.reveal.push(option);
                        }
                        changed = FormChange::at(choice_path);
                    }
                }
                if choice.reveal.is_empty() {
                    ui.label(theme::muted("nothing").size(12.5));
                }
            });
        });
        form::kv(ui, "warning", |ui| match &mut choice.warn {
            None => {
                if ui
                    .add(
                        egui::Label::new(theme::muted("none").size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Warn before the choice")
                    .clicked()
                {
                    choice.warn = Some(String::new());
                    changed = FormChange::at(choice_path);
                }
            }
            Some(warn) => {
                if forms::text_of(ui, cx, warn, &format!("{choice_path}/warn")).is_some() {
                    if warn.is_empty() {
                        choice.warn = None;
                    }
                    changed = FormChange::at(choice_path);
                }
            }
        });
        form::kv(ui, "arrow", |ui| match choice.to.to() {
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
    let Some(Node::Dialogue(dialogue)) = doc.nodes().get(name).cloned() else {
        return;
    };
    let count = dialogue.choices.len();
    let Some(choice) = dialogue.choices.get(index) else {
        node_inspector(editor, ui, doc, name);
        return;
    };
    let label_text = spans::plain(&choice.label);
    breadcrumb(
        ui,
        &format!(
            "{} › {name} › {}",
            doc.at.title(),
            words::clip(&label_text, 40)
        ),
    );
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(ScriptDef::TABLE);
    let path = format!("/nodes/{name}");
    let mut draft = dialogue.clone();
    let mut actions = Vec::new();
    let outcome = {
        let mut cx = doc.form_cx(editor, root, &mut actions);
        let at = ChoiceAt {
            path: &path,
            index,
            count,
            doc,
        };
        choice_card(ui, &mut cx, &mut draft.choices[index], &at)
    };
    let ChoiceOutcome {
        mut changed,
        menu,
        go,
    } = outcome;
    for action in actions {
        act(editor, action, doc);
    }
    let mut label = format!("Edit a choice in {}", doc.at.title());
    if let Some(removed) = menu.remove {
        draft.choices.remove(removed);
        changed = FormChange::at(&path);
        label = format!("Remove a choice in {}", doc.at.title());
    }
    if let Some((a, b)) = menu.swap {
        draft.choices.swap(a, b);
        changed = FormChange::at(&path);
    }
    if let Some(target) = go {
        select_nodes(editor, doc, &[target]);
        return;
    }
    if changed.is_some() {
        let mut script = doc.edited();
        script.nodes.insert(name.to_owned(), Node::Dialogue(draft));
        write(editor, doc, script, label);
    }
}

fn speaker_rows(
    ui: &mut Ui,
    cx: &mut FormCx,
    line: &mut Line,
    path: &str,
    doc: &ScriptDoc,
) -> FormChange {
    let mut changed = FormChange::default();
    let reader = cx.project.reader();
    let current = SpeakerKind::of(line.by);
    let mut picked = current;
    form::kv(ui, "by", |ui| {
        controls::combo(ui.id().with(path).with("speaker"))
            .selected_text(theme::mono(current.name()).size(13.0))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for &option in SpeakerKind::options(doc.kind) {
                    ui.selectable_value(&mut picked, option, option.name());
                }
            });
    });
    if picked != current {
        let npc = cx
            .speaker
            .as_deref()
            .and_then(|npc| reader.id_of::<NpcDef>(npc))
            .or_else(|| {
                let nearest = reader
                    .paths_of::<NpcDef>()
                    .iter()
                    .min_by_key(|npc| cx.seen_from_here(npc).0)?;
                reader.id_of::<NpcDef>(nearest)
            });
        let by = match picked {
            SpeakerKind::Npc => npc.map(Speaker::Npc),
            SpeakerKind::Object => Some(Speaker::Object),
            SpeakerKind::Narrator => Some(Speaker::Narrator),
            SpeakerKind::Player => Some(Speaker::Player),
        };
        match by {
            Some(by) => {
                line.by = by;
                line.face = None;
                changed = FormChange::at(path);
            }
            None => {
                cx.state
                    .errors
                    .insert(path.to_owned(), "the game has no npc to speak".to_owned());
            }
        }
    }
    let Speaker::Npc(npc) = line.by else {
        return changed;
    };
    let mut npc_path = reader.path_of(npc).unwrap_or_default().to_owned();
    form::kv(ui, "  npc", |ui| {
        if picker::row(ui, cx, NpcDef::TABLE, &mut npc_path, &format!("{path}/npc"))
            && let Some(picked) = reader.id_of::<NpcDef>(&npc_path)
        {
            line.by = Speaker::Npc(picked);
            if nodes::bust_file(cx.project, line.by, line.face.as_ref()).is_none() {
                line.face = None;
            }
            changed = FormChange::at(path);
        }
    });
    cx.speaker = Some(npc_path);
    form::kv(ui, "  face", |ui| {
        let ctx = ui.ctx().clone();
        if let Some(file) = nodes::bust_file(cx.project, line.by, line.face.as_ref())
            && let Some(thumb) = cx.state.thumbnails.image(&ctx, cx.files, &file)
        {
            ui.add(
                egui::Image::from_texture((thumb.texture.id(), egui::vec2(26.0, 26.0)))
                    .uv(thumbs::square_uv(thumb.size))
                    .corner_radius(4.0),
            );
        }
        changed |= forms::of_type(ui, cx, &mut line.face, &format!("{path}/face"));
    });
    changed
}

fn act(editor: &mut Editor, action: FormAction, doc: &ScriptDoc) {
    row_form::act(editor, action, ScriptDef::TABLE, &doc.at.key);
}

/// Carries out what a node's card asked for, but for the node it asks to go to, which the card
/// goes to once it is done with the node it shows.
fn act_but_go(editor: &mut Editor, actions: Vec<FormAction>, doc: &ScriptDoc) -> Option<String> {
    let mut go_to = None;
    for action in actions {
        match action {
            FormAction::OpenNode { name } => go_to = Some(name),
            other => act(editor, other, doc),
        }
    }
    go_to
}
