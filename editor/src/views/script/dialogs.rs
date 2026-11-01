use super::graph::ScriptGraphAction;
use super::{
    doc::{ScriptDoc, ScriptRow},
    edit::{act, rename_node},
    nodes,
};
use crate::app::Editor;
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::egui::RichText;
use ui::input;
use ui::input::InputField;
use ui::theme;

pub struct RenameNodeDialog {
    pub at: ScriptRow,
    pub old: String,
    pub draft: String,
}

pub struct LinkNodeDialog {
    pub at: ScriptRow,
    pub from: String,
    pub search: String,
    pub port: usize,
}

pub fn rename_node_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: RenameNodeDialog,
) -> Option<RenameNodeDialog> {
    let doc = ScriptDoc::load(editor, &dialog.at)?;
    let mut outcome: Option<Option<String>> = None;
    dialog::card(ctx, "rename-node", 360.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(RichText::new("Rename node").strong().size(18.0));
        ui.add_space(6.0);
        ui.label(theme::muted(format!("{} becomes:", dialog.old)).size(13.5));
        let response = InputField::new(&mut dialog.draft).show(ui);
        if !response.has_focus() {
            response.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(dialog.draft.chars().count()),
                )));
            state.store(ui.ctx(), response.id);
        }
        let taken = dialog.draft != dialog.old && doc.nodes().contains_key(&dialog.draft);
        if taken {
            ui.colored_label(theme::DANGER, "that name is taken");
        }
        let ok = !dialog.draft.trim().is_empty()
            && !taken
            && dialog
                .draft
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("Rename")).clicked() || (ok && pressed.confirm)
            {
                outcome = Some(Some(dialog.draft.clone()));
            }
            if ui.button("Cancel").clicked() || pressed.cancel {
                outcome = Some(None);
            }
        });
    });
    match outcome {
        None => Some(dialog),
        Some(None) => None,
        Some(Some(new)) => {
            if new != dialog.old {
                rename_node(editor, &doc, &dialog.old, &new);
            }
            None
        }
    }
}

pub fn link_node_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: LinkNodeDialog,
) -> Option<LinkNodeDialog> {
    let doc = ScriptDoc::load(editor, &dialog.at)?;
    let ports = doc
        .nodes()
        .get(&dialog.from)
        .map(nodes::ports)
        .unwrap_or_default();
    let mut outcome: Option<Option<String>> = None;
    dialog::card(ctx, "link-node", 360.0, |ui| {
        let pressed = dialog::keys(ui);
        ui.label(
            RichText::new(format!("Link {} to…", dialog.from))
                .strong()
                .size(18.0),
        );
        ui.add_space(6.0);
        let response = input::search(ui, &mut dialog.search, "node name", None);
        input::default_focus(ui, &response);
        let needle = dialog.search.to_lowercase();
        if ports.len() > 1 {
            controls::combo("link-port")
                .selected_text(
                    ports
                        .get(dialog.port)
                        .map(|p| p.label.clone())
                        .unwrap_or_default(),
                )
                .show_ui(ui, |ui| {
                    for (index, port) in ports.iter().enumerate() {
                        ui.selectable_value(&mut dialog.port, index, &port.label);
                    }
                });
        }
        let matches: Vec<&String> = doc
            .nodes()
            .keys()
            .filter(|name| *name != &dialog.from && name.to_lowercase().contains(&needle))
            .collect();
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for name in &matches {
                    if ui.selectable_label(false, *name).clicked() {
                        outcome = Some(Some((*name).clone()));
                    }
                }
            });
        if pressed.confirm
            && let Some(name) = matches.first()
        {
            outcome = Some(Some((*name).clone()));
        }
        if pressed.cancel {
            outcome = Some(None);
        }
    });
    match outcome {
        None => Some(dialog),
        Some(None) => None,
        Some(Some(to)) => {
            if let Some(port) = ports.get(dialog.port) {
                let action = ScriptGraphAction::Connect {
                    from: dialog.from,
                    exit: port.exit,
                    to,
                };
                act(editor, &doc, action);
            }
            None
        }
    }
}
