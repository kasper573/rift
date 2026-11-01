use super::graph::{NodePaletteOpen, ScriptGraphAction};
use super::{
    doc::ScriptDoc,
    edit::{free_port_link, link_by_name},
    nodes,
};
use crate::app::{BottomTab, Editor};
use crate::keymap::EditorKey;
use ui::egui;
use ui::egui::{Ui, vec2};

#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptKeys {
    pub fire: bool,
    pub tab: bool,
    pub outline: bool,
    pub delete: bool,
    pub rename: bool,
    pub duplicate: bool,
    pub enter: bool,
    pub ctrl_enter: bool,
    pub link: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl ScriptKeys {
    pub fn take(ctx: &egui::Context) -> ScriptKeys {
        ctx.input_mut(|i| {
            let arrow = EditorKey::Move.take_arrow(i);
            ScriptKeys {
                fire: EditorKey::RunScript.consume(i),
                tab: EditorKey::Palette.consume(i),
                outline: EditorKey::Outline.consume(i),
                delete: EditorKey::Delete.consume(i),
                rename: EditorKey::Rename.consume(i),
                duplicate: EditorKey::Duplicate.consume(i),
                enter: EditorKey::Confirm.consume(i),
                ctrl_enter: EditorKey::AddChoice.consume(i),
                link: EditorKey::Link.consume(i),
                up: arrow == Some([0, -1]),
                down: arrow == Some([0, 1]),
                left: arrow == Some([-1, 0]),
                right: arrow == Some([1, 0]),
            }
        })
    }
}

pub(crate) fn handle(
    editor: &mut Editor,
    ui: &mut Ui,
    doc: &ScriptDoc,
    actions: &mut Vec<ScriptGraphAction>,
) {
    let at = doc.at.clone();
    let keys = editor.views.script.keys;
    if editor
        .views
        .script
        .view_mut(&at, &editor.user, editor.project.as_ref())
        .palette
        .is_some()
    {
        return;
    }
    if keys.fire {
        editor.bottom_tab = BottomTab::Run;
        editor.views.script.fire(&at);
        return;
    }
    let views = &mut editor.views.script;
    let view = views.view_mut(&at, &editor.user, editor.project.as_ref());
    let in_outline = view.outline;
    if keys.outline {
        view.toggle_outline();
    }
    if in_outline {
        return;
    }
    let selected = view.selected.clone();
    let first = selected.first().cloned();
    if keys.tab {
        let link = first.as_ref().and_then(|name| free_port_link(doc, name));
        let beside_node = first
            .as_ref()
            .and_then(|name| view.rects.get(name))
            .zip(view.to_global)
            .map(|(rect, to_global)| to_global * rect.right_top() + vec2(16.0, 0.0));
        actions.push(ScriptGraphAction::Palette(NodePaletteOpen {
            at: beside_node
                .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                .unwrap_or_else(|| ui.max_rect().center()),
            placed: false,
            link,
        }));
    }
    if keys.delete && !selected.is_empty() {
        actions.push(ScriptGraphAction::Delete(selected.clone()));
    }
    if keys.rename
        && let Some(name) = &first
    {
        actions.push(ScriptGraphAction::Rename(name.clone()));
    }
    if keys.duplicate && !selected.is_empty() {
        actions.push(ScriptGraphAction::Duplicate(selected.clone()));
    }
    if let Some(name) = &first
        && doc.nodes().get(name).is_some_and(nodes::is_dialogue)
    {
        if keys.ctrl_enter {
            actions.push(ScriptGraphAction::AddChoice(name.clone()));
        } else if keys.enter {
            actions.push(ScriptGraphAction::AddLine(name.clone()));
        }
    }
    if keys.link
        && let Some(name) = first
    {
        link_by_name(editor, &at, name);
    }
}
