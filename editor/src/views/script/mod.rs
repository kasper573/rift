pub mod callout;
pub mod dialogs;
pub mod doc;
pub mod edit;
pub mod graph;
pub mod inspect;
pub mod keys;
pub mod nodes;
pub mod outline;
pub mod palette;
pub mod panel;
pub mod run;
pub mod session;

pub use dialogs::{LinkNodeDialog, RenameNodeDialog, link_node_dialog, rename_node_dialog};
pub use doc::{ScriptDoc, ScriptPath, ScriptRef, ScriptRow};
use edit::{act, is_node_selection};
pub use edit::{add_node, delete_nodes_now, select_nodes};
use keys::ScriptKeys;
use session::{PendingScriptAction, snarl_id};
pub use session::{ScriptSessionId, ScriptViewState, ScriptViews};

use std::collections::{HashMap, HashSet};
use ui::chip;
use ui::egui;
use ui::theme;
use ui::words;

use engine::core::content::ContentFile;
use engine::core::script::RunnerKindDef;
use engine::systems::script::ScriptDef;
use graph::{ScriptGraphAction, ScriptGraphLayout};
use ui::egui::emath::TSTransform;
use ui::egui::{Pos2, Rect, Ui, vec2};
use ui::egui_snarl::ui::get_selected_nodes;

use crate::app::{BottomTab, Editor};
use crate::keymap::EditorKey;
use crate::project::RowSelection;
use crate::tabs::{DockRequest, EditorTab};

pub(crate) fn content_generation(editor: &Editor) -> u64 {
    editor
        .project
        .as_ref()
        .map_or(0, |project| project.generation)
}

pub(crate) fn sync_graph(editor: &mut Editor, doc: &ScriptDoc) {
    let generation = content_generation(editor);
    let Editor {
        user,
        project,
        views,
        ..
    } = editor;
    let Some(project) = project.as_ref() else {
        return;
    };
    views
        .script
        .view_mut(&doc.at, user, Some(project))
        .graph
        .sync(doc, &doc.text(project), generation);
}

pub(crate) fn sync_with_content(editor: &mut Editor) {
    let generation = content_generation(editor);
    let views = &mut editor.views.script;
    if views
        .run
        .as_ref()
        .is_some_and(|run| run.generation != generation)
    {
        views.run = None;
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    if crate::views::read_shown::<ScriptDef>(editor, ui, &at.key).is_none() {
        return;
    }
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    editor.views.script.shown_rect = Some(ui.max_rect());
    if let Some(pending) = editor.views.script.take_pending(at) {
        match pending {
            PendingScriptAction::Connect { from, output, to } => {
                let exit = doc
                    .nodes()
                    .get(&from)
                    .and_then(|node| nodes::exit_called(node, &output));
                if let Some(exit) = exit {
                    act(editor, &doc, ScriptGraphAction::Connect { from, exit, to })
                }
            }
        }
    }
    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
        editor
            .dock_requests
            .push(DockRequest::Focus(EditorTab::Script(at.clone())));
    }
    sync_with_content(editor);
    sync_graph(editor, &doc);
    let mut actions = Vec::new();
    keys::handle(editor, ui, &doc, &mut actions);
    status(editor, &doc);
    ui::panel::left_scrolled(ui, "script", |ui| inspect::script_row(editor, ui, &doc));
    let (table, key) = at.row();
    let selection = if editor.selection.table == table
        && editor.selection.keys.first().is_some_and(|k| *k == key)
    {
        editor.selection.clone()
    } else {
        RowSelection::row(&table, &key)
    };
    ui::panel::inspector(ui, editor, "script", |editor, ui| {
        inspect::show(editor, ui, &doc, &selection)
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            if editor
                .views
                .script
                .view_mut(at, &editor.user, editor.project.as_ref())
                .outline
            {
                outline::show(editor, ui, &doc, &mut actions);
            } else {
                canvas(editor, ui, &doc, &mut actions);
            }
            view_switch(editor, ui, at);
        });
    palette::show(editor, ui, &doc);
    for action in actions {
        let Some(doc) = ScriptDoc::load(editor, at) else {
            break;
        };
        act(editor, &doc, action);
    }
    editor.views.script.keys = ScriptKeys::default();
    save_layout(editor, ui, at);
}

pub fn bottom(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow, tab: BottomTab) {
    let Some(doc) = ScriptDoc::load(editor, at) else {
        return;
    };
    sync_with_content(editor);
    panel::show(editor, ui, &doc, tab);
}

/// Canvas and Outline, at the top right of the view they switch between, in a layer above the
/// canvas's, which would otherwise take their clicks.
pub(crate) fn view_switch(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    let view = editor
        .views
        .script
        .view_mut(at, &editor.user, editor.project.as_ref());
    let mut switch = false;
    egui::Area::new(ui.id().with("view-switch"))
        .order(egui::Order::Foreground)
        .fixed_pos(ui.max_rect().right_top() + vec2(-10.0, 10.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, outline) in [("Canvas", false), ("Outline", true)] {
                    let tip = format!("{} switches between them", EditorKey::Outline.name());
                    if chip::mode(ui, label, view.outline == outline)
                        .on_hover_text(tip)
                        .clicked()
                        && view.outline != outline
                    {
                        switch = true;
                    }
                }
            });
        });
    if switch {
        view.toggle_outline();
    }
}

pub(crate) fn canvas(
    editor: &mut Editor,
    ui: &mut Ui,
    doc: &ScriptDoc,
    actions: &mut Vec<ScriptGraphAction>,
) {
    let at = doc.at.clone();
    let (lit, current): (HashSet<String>, Option<String>) = editor
        .views
        .script
        .run_of(&at)
        .map(|run| {
            (
                run.visited.iter().cloned().collect(),
                run.visited.last().cloned(),
            )
        })
        .unwrap_or_default();
    let selected_before = {
        let views = &mut editor.views.script;
        let view = views.view_mut(&at, &editor.user, editor.project.as_ref());
        let selection = &editor.selection;
        if selection.table == ScriptDef::TABLE
            && selection.keys == [at.key.as_str()]
            && is_node_selection(selection)
        {
            let mut names: Vec<String> = selection
                .parts
                .iter()
                .filter_map(|part| part.strip_prefix("node:"))
                .map(str::to_owned)
                .collect();
            names.sort();
            view.selected = names;
        }
        view.selected.clone()
    };
    let id = snarl_id(&at);
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let (dragged, to_global, mut taken, clicked, rects) = {
        let views = &mut editor.views.script;
        let view = views.view_mut(&at, &editor.user, Some(project));
        let chosen: HashSet<String> = selected_before.iter().cloned().collect();
        let mut viewer = graph::ScriptGraphViewer {
            doc,
            text: doc.text(project),
            ports: graph::ports_of(doc),
            lit: &lit,
            current: current.as_deref(),
            selected: &chosen,
            actions: Vec::new(),
            clicks: Vec::new(),
            to_global: TSTransform::IDENTITY,
            rects: HashMap::new(),
            fit: None,
        };
        if view.to_global.is_none() {
            let placed = view.graph.positions();
            let spots: Vec<Pos2> = if placed.is_empty() {
                vec![graph::FIRST]
            } else {
                placed.into_values().collect()
            };
            let bounds = spots.into_iter().fold(Rect::NOTHING, |bounds, at| {
                bounds.union(Rect::from_min_size(at, vec2(graph::CARD + 20.0, 120.0)))
            });
            viewer.fit = Some((bounds, ui.max_rect()));
        }
        let graph = &mut view.graph;
        let before = graph.node_spots();
        graph::widget(id).show(&mut graph.snarl, &mut viewer, ui);
        let dragged = graph.node_spots() != before;
        (
            dragged,
            viewer.to_global,
            viewer.actions,
            viewer.clicks,
            viewer.rects,
        )
    };
    header_chip(editor, ui, doc);
    editor
        .views
        .script
        .view_mut(&at, &editor.user, editor.project.as_ref())
        .rects = rects;
    run::conversation_box(editor, ui, doc);
    actions.append(&mut taken);
    let selected: Vec<String> = {
        let view = editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref());
        let graph = &view.graph;
        let mut marked: Vec<String> = get_selected_nodes(id, ui.ctx())
            .iter()
            .filter_map(|id| graph.snarl.get_node(*id).map(|node| node.name.clone()))
            .collect();
        marked.sort();
        if dragged {
            view.layout_dirty = true;
        }
        view.to_global = Some(to_global);
        let mut names = if marked != view.marked {
            view.marked = marked.clone();
            marked
        } else {
            selected_before.clone()
        };
        for graph::NodeClick { name, extend } in clicked {
            if !extend {
                names.clear();
            }
            if !names.contains(&name) {
                names.push(name);
            }
            names.sort();
        }
        names
    };
    callout::show(editor, ui, &at);
    if selected != selected_before {
        editor
            .views
            .script
            .view_mut(&at, &editor.user, editor.project.as_ref())
            .selected = selected.clone();
        if !selected.is_empty() {
            select_nodes(editor, doc, &selected);
        }
    }
}

pub(crate) fn header_chip(editor: &Editor, ui: &Ui, doc: &ScriptDoc) {
    let text = match editor.views.script.run_of(&doc.at) {
        Some(_) => format!("{} · running with a pretend subject", doc.at.title()),
        None => chip_title(editor, doc),
    };
    let above_the_canvas = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.id().with("header-chip"),
        ))
        .with_clip_rect(ui.max_rect());
    chip::floating(
        &above_the_canvas,
        ui.max_rect().min + vec2(16.0, 12.0),
        egui::Align2::LEFT_TOP,
        &text,
        theme::PANEL,
        theme::TEXT,
        12.5,
    );
}

pub(crate) fn chip_title(editor: &Editor, doc: &ScriptDoc) -> String {
    let count = doc.nodes().len();
    let mut text = format!(
        "{} · runs on {} · {count} node{}",
        doc.at.title(),
        doc.kind.name(),
        words::plural(count)
    );
    if editor.shapes.is_fixture(ScriptDef::TABLE, &doc.at.key) {
        text.push_str(" · from New Game");
    }
    text
}

pub(crate) fn status(editor: &mut Editor, doc: &ScriptDoc) {
    let at = doc.at.clone();
    let view = editor.views.script.view(&at);
    let outline = view.is_some_and(|view| view.outline);
    let palette = view.is_some_and(|view| view.palette.is_some());
    let selected = view.map(|view| view.selected.clone()).unwrap_or_default();
    let head = match selected.as_slice() {
        [one] if !outline && !palette => format!("{} › {one}", at.title()),
        _ => {
            let mode = if outline {
                " · outline"
            } else if palette {
                " · palette"
            } else {
                ""
            };
            format!("{} › script{mode}", at.title())
        }
    };
    let refused = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(&at))
        .map(|callout| callout.rule.clone());
    let hint = if let Some(rule) = refused {
        format!("refused · {rule}  nothing changed")
    } else if let Some(run) = editor.views.script.run_of(&at) {
        let waiting = run
            .visited
            .last()
            .map(|node| format!("waiting at {node}"))
            .unwrap_or_else(|| "ended".to_owned());
        format!(
            "Run · 1 flow · {waiting}   {} pick a choice · {} stops",
            EditorKey::Pick.name(),
            EditorKey::Cancel.name()
        )
    } else if palette {
        format!(
            "{} choose · {} opens a system · {} places it",
            EditorKey::Move.name(),
            EditorKey::Move.name(),
            EditorKey::Confirm.name()
        )
    } else if outline {
        format!(
            "{} switches to the canvas, keeping the selection",
            EditorKey::Outline.name()
        )
    } else {
        match selected.as_slice() {
            [one] => doc.nodes().get(one).map_or_else(String::new, nodes::hint),
            [] => format!(
                "{} opens the palette · drag from a dot to link · {} removes",
                EditorKey::Palette.name(),
                EditorKey::Delete.name()
            ),
            many => format!(
                "{} nodes · drag moves them together · {} removes them",
                many.len(),
                EditorKey::Delete.name()
            ),
        }
    };
    editor.status.left.push(head);
    editor.status.left.push(hint);
}

pub(crate) fn save_layout(editor: &mut Editor, ui: &Ui, at: &ScriptRow) {
    if ui.input(|input| input.pointer.any_down()) {
        return;
    }
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(view) = editor.views.script.existing_view_mut(at) else {
        return;
    };
    if !view.layout_dirty {
        return;
    }
    view.layout_dirty = false;
    let layout = ScriptGraphLayout::of(&view.graph, view.outline);
    if let Err(error) = editor.user.save_layout(&project.root, &at.slug(), &layout) {
        editor.toast(format!("The graph layout was not saved: {error}"));
    }
}
