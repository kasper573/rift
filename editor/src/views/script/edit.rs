use super::graph::ScriptGraphAction;
use super::{
    callout::{ScriptCallout, beside},
    dialogs::{LinkNodeDialog, RenameNodeDialog},
    doc::{ScriptDoc, ScriptRef, ScriptRow},
    graph, nodes, palette,
};
use crate::app::Editor;
use crate::modal::{ConfirmAction, ConfirmDialog, ModalDialog};
use crate::project::{ContentEdit, RowSelection};
use engine::core::content::ContentFile;
use engine::core::script::{Arrow, Choice, Exit, RunnerKindDef};
use engine::systems::script::{Node, ScriptDef};
use engine::systems::text::plain;
use std::collections::HashSet;
use std::time::Instant;
use ui::egui::Pos2;

pub fn free_port_link(doc: &ScriptDoc, name: &str) -> Option<(String, Exit)> {
    let node = doc.nodes().get(name)?;
    nodes::ports(node)
        .into_iter()
        .find(|port| port.target.is_none())
        .map(|port| (name.to_owned(), port.exit))
}

pub fn link_by_name(editor: &mut Editor, at: &ScriptRow, from: String) {
    editor.dialogs.push(ModalDialog::LinkNode(LinkNodeDialog {
        at: at.clone(),
        from,
        search: String::new(),
        port: 0,
    }));
}

pub fn act(editor: &mut Editor, doc: &ScriptDoc, action: ScriptGraphAction) {
    let at = doc.at.clone();
    let mut script = doc.edited();
    match action {
        ScriptGraphAction::Connect { from, exit, to } => {
            if let Some(node) = script.nodes.get_mut(&from) {
                nodes::set_arrow(node, exit, Some(&to));
            }
            let label = format!("Link {from} to {to} in {}", at.title());
            if !write(editor, doc, script, label)
                && let Some(callout) = editor.views.script.callout.as_mut()
            {
                callout.node = from;
                callout.to = Some(to);
            }
        }
        ScriptGraphAction::Disconnect { from, exit } => {
            if let Some(node) = script.nodes.get_mut(&from) {
                nodes::set_arrow(node, exit, None);
            }
            let label = format!("Unlink {from} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::DropInputs(to) => {
            nodes::unlink(&mut script.nodes, &to);
            let label = format!("Unlink arrows into {to} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::Delete(names) => delete_nodes(editor, doc, names),
        ScriptGraphAction::Rename(name) => {
            editor
                .dialogs
                .push(ModalDialog::RenameNode(RenameNodeDialog {
                    at,
                    old: name.clone(),
                    draft: name,
                }));
        }
        ScriptGraphAction::Duplicate(names) => duplicate(editor, doc, &names),
        ScriptGraphAction::Palette(open) => {
            editor
                .views
                .script
                .view_mut(&at, &editor.user, editor.project.as_ref())
                .palette = Some(palette::NodePalette::new(open));
        }
        ScriptGraphAction::AddLine(name) => {
            if let Some(Node::Dialogue(dialogue)) = script.nodes.get_mut(&name) {
                dialogue
                    .lines
                    .push(nodes::blank_line(doc.default_speaker()));
            }
            let label = format!("Add a line to {name} in {}", at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::EditLine { name, index, text } => {
            if let Some(Node::Dialogue(dialogue)) = script.nodes.get_mut(&name)
                && let Some(line) = dialogue.lines.get_mut(index)
            {
                line.text = text;
            }
            let label = format!("Edit line {} of {name} · {}", index + 1, at.title());
            write(editor, doc, script, label);
        }
        ScriptGraphAction::AddChoice(name) => {
            if let Some(node) = script.nodes.get_mut(&name) {
                add_choice(node);
            }
            let label = format!("Add a choice to {name} in {}", at.title());
            write(editor, doc, script, label);
        }
    }
}

/// Gives a dialogue a choice more, its plain next given up to the choices.
pub fn add_choice(node: &mut Node) {
    let Node::Dialogue(dialogue) = node else {
        return;
    };
    dialogue.next.point(None);
    dialogue.choices.push(Choice {
        label: vec![plain("…")],
        requires: None,
        reveal: Vec::new(),
        warn: None,
        to: Arrow::default(),
    });
}

/// Copies each of the nodes `names`, each copy led to from a free output of its original, as one
/// step; a node with no free output is left uncopied.
pub fn duplicate(editor: &mut Editor, doc: &ScriptDoc, names: &[String]) {
    let at = doc.at.clone();
    let mut script = doc.edited();
    let mut copies: Vec<(String, String)> = Vec::new();
    for name in names {
        let Some(mut copy) = doc.nodes().get(name).cloned() else {
            continue;
        };
        let Some(free) = nodes::ports(&copy)
            .into_iter()
            .find(|port| port.target.is_none())
        else {
            editor.toast(format!("{name} has no free output to lead to its copy"));
            continue;
        };
        for (_, arrow) in copy.exits_mut() {
            arrow.point(None);
        }
        let fresh = nodes::free_name(&script.nodes, name);
        script.nodes.insert(fresh.clone(), copy);
        if let Some(source) = script.nodes.get_mut(name) {
            nodes::set_arrow(source, free.exit, Some(&fresh));
        }
        copies.push((name.clone(), fresh));
    }
    let label = match copies.as_slice() {
        [] => return,
        [(name, _)] => format!("Duplicate {name} in {}", at.title()),
        many => format!("Duplicate {} nodes in {}", many.len(), at.title()),
    };
    if write(editor, doc, script, label) {
        let view = editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref());
        for (name, fresh) in &copies {
            let at = beside(&view.graph, name);
            view.place_node(fresh, at);
        }
        let fresh: Vec<String> = copies.into_iter().map(|(_, fresh)| fresh).collect();
        select_nodes(editor, doc, &fresh);
    }
}

pub fn remove_nodes(doc: &ScriptDoc, names: &[String]) -> ScriptDef {
    let mut script = doc.edited();
    for name in names {
        script.nodes.remove(name);
        nodes::unlink(&mut script.nodes, name);
    }
    script
}

pub fn delete_nodes(editor: &mut Editor, doc: &ScriptDoc, names: Vec<String>) {
    let script = remove_nodes(doc, &names);
    let reachable = nodes::reachable(&script.nodes);
    let were_reachable = nodes::reachable(doc.nodes());
    let newly_stranded: Vec<String> = script
        .nodes
        .iter()
        .filter(|(name, node)| {
            !nodes::is_event(node) && !reachable.contains(*name) && were_reachable.contains(*name)
        })
        .map(|(name, _)| name.clone())
        .collect();
    if !newly_stranded.is_empty() {
        editor.dialogs.push(ModalDialog::Confirm(ConfirmDialog {
            title: format!("Delete {}?", names.join(", ")),
            body: format!(
                "No flow would reach {} afterwards. Delete those too, or link them elsewhere first.",
                newly_stranded.join(", ")
            ),
            label: format!("Delete {} nodes", names.len() + newly_stranded.len()),
            compare: None,
            action: ConfirmAction::DeleteScriptNodes {
                script: doc.at.clone(),
                names: names.iter().chain(newly_stranded.iter()).cloned().collect(),
            },
        }));
        return;
    }
    let label = match names.as_slice() {
        [one] => format!("Delete {one} in {}", doc.at.title()),
        many => format!("Delete {} nodes in {}", many.len(), doc.at.title()),
    };
    if write(editor, doc, script, label) {
        select_nodes(editor, doc, &[]);
    }
}

pub fn delete_nodes_now(editor: &mut Editor, at: &ScriptRow, names: Vec<String>) {
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    let script = remove_nodes(&doc, &names);
    let label = format!("Delete {} nodes in {}", names.len(), at.title());
    if write(editor, &doc, script, label) {
        select_nodes(editor, &doc, &[]);
    }
}

pub fn add_node(
    editor: &mut Editor,
    doc: &ScriptDoc,
    kind: &str,
    at: Option<Pos2>,
    link: Option<(String, Exit)>,
) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let nearby = crate::schema::Nearby::of(project, Some(&doc.file));
    let Some(mut node) = nodes::fresh(&doc.reader, kind, doc.kind, nearby, doc.default_speaker())
    else {
        editor.toast(format!("No {kind} node can be made here"));
        return;
    };
    if nodes::is_event(&node) {
        let used: HashSet<&str> = doc.nodes().values().filter_map(nodes::event_name).collect();
        let Some(event) =
            nodes::free_event(&doc.reader, &project.catalogue, doc.kind, &used, nearby)
        else {
            editor.toast(format!(
                "Every event a {} script answers already starts a flow here",
                doc.kind.name()
            ));
            return;
        };
        node = event;
    }
    let base = if nodes::is_dialogue(&node) {
        &doc.at.key
    } else {
        kind
    };
    let name = nodes::free_name(doc.nodes(), base);
    let mut script = doc.edited();
    script.nodes.insert(name.clone(), node);
    if let Some((from, exit)) = &link
        && let Some(source) = script.nodes.get_mut(from)
    {
        nodes::set_arrow(source, *exit, Some(&name));
    }
    let script_row = doc.at.clone();
    let label = match &link {
        Some((from, _)) => format!("Add {kind} after {from} in {}", script_row.title()),
        None => format!("Add {kind} to {}", script_row.title()),
    };
    if write(editor, doc, script, label) {
        let view = editor
            .views
            .script
            .view_mut(&script_row, &editor.user, editor.project.as_ref());
        let graph_pos = match (at, view.to_global) {
            (Some(at), Some(to_global)) => to_global.inverse() * at,
            (Some(at), None) => at,
            (None, _) => link
                .as_ref()
                .map_or(graph::FIRST, |(from, _)| beside(&view.graph, from)),
        };
        view.place_node(&name, graph_pos);
        select_nodes(editor, doc, &[name]);
    }
}

pub fn write(editor: &mut Editor, doc: &ScriptDoc, script: ScriptDef, label: String) -> bool {
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut edit = ContentEdit::new(label);
    if let Err(reason) = edit.write_content(project, &doc.at.key, &script) {
        editor.toast(format!("{}: {reason}", edit.label));
        return false;
    }
    let written = editor.apply(edit);
    editor.views.script.callout = (!written)
        .then_some(editor.last_refusal.as_ref())
        .flatten()
        .map(|refusal| {
            let named = ScriptRef::parse(&refusal.pointer)
                .node
                .filter(|name| doc.nodes().contains_key(name));
            let selected = editor
                .views
                .script
                .view(&doc.at)
                .and_then(|view| view.selected.first().cloned());
            ScriptCallout {
                at: doc.at.clone(),
                node: named.or(selected).unwrap_or_default(),
                to: None,
                rule: refusal.problem.rule.replace('_', " "),
                text: format!(
                    "{}; expected {}.",
                    refusal.problem.found, refusal.problem.expected
                ),
                since: Instant::now(),
            }
        });
    written
}

pub fn rename_node(editor: &mut Editor, doc: &ScriptDoc, old: &str, new: &str) {
    let mut script = doc.edited();
    nodes::rename(&mut script.nodes, old, new);
    let at = doc.at.clone();
    let label = format!("Rename {old} to {new} in {}", at.title());
    if write(editor, doc, script, label) {
        editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref())
            .rename_node(old, new);
        select_nodes(editor, doc, &[new.to_owned()]);
    }
}

pub fn select_nodes(editor: &mut Editor, doc: &ScriptDoc, names: &[String]) {
    editor.select(RowSelection::parts(
        ScriptDef::TABLE,
        &doc.at.key,
        names.iter().map(|name| format!("node:{name}")).collect(),
    ));
    editor
        .views
        .script
        .view_mut(&doc.at, &editor.user, editor.project.as_ref())
        .selected = names.to_vec();
}

pub fn is_node_selection(selection: &RowSelection) -> bool {
    !selection.parts.is_empty()
        && selection
            .parts
            .iter()
            .all(|part| part.starts_with("node:") || part.starts_with("choice:"))
}
