//! A face of a bust picked on its page: its name, file and size, the lines asking for the bust's
//! own faces, and the dialog removing one.

use ui::controls;
use ui::dialog;
use ui::egui;
use ui::form;
use ui::input;
use ui::panel;
use ui::theme;

use engine::core::content::ContentFile;
use engine::systems::dialogue::bust::BustDef;
use engine::systems::script::ScriptDef;
use ui::egui::{RichText, Ui};

use super::faces::{self, BustFace, FaceLine};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::RowSelection;
use crate::shell::widgets;
use crate::tabs::shown_path;
use ui::input::InputField;

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(key) = selection.keys.first().cloned() else {
        return;
    };
    let Some(part) = selection.parts.first().cloned() else {
        return;
    };
    let Some(row) = crate::views::read_shown::<BustDef>(editor, ui, &key) else {
        return;
    };
    super::ensure_view(editor, &key);
    let faces = faces::faces(&row);
    if let Some(face) = faces.iter().find(|face| face.part() == part).cloned() {
        egui::ScrollArea::vertical()
            .id_salt(("bust-inspect", &key))
            .show(ui, |ui| {
                face_card(editor, ui, &key, &face, &faces);
            });
    }
}

fn face_card(editor: &mut Editor, ui: &mut Ui, key: &str, face: &BustFace, faces: &[BustFace]) {
    ui.label(
        theme::mono(format!("{} › {}", shown_path(key), face.name))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(4.0);
    let ctx = ui.ctx().clone();
    let sizes: Vec<(String, Option<[usize; 2]>)> = faces
        .iter()
        .map(|other| {
            let size = editor.thumb(&ctx, &other.file).map(|thumb| thumb.size);
            (other.name.clone(), size)
        })
        .collect();
    let own_size = sizes
        .iter()
        .find(|(name, _)| *name == face.name)
        .and_then(|(_, size)| *size);
    let mut rename_to: Option<String> = None;
    let mut remove_with: Option<String> = None;
    let lines = faces::lines(editor, key);
    let uses_of_face = lines.iter().filter(|line| line.face == face.name).count();
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let kind = if face.own { "" } else { "generic" };
        panel::header(ui, &face.name, kind, |ui| {
            if face.own && ui.button("Remove this face…").clicked() {
                remove_with = Some(String::new());
                ui.close();
            }
        });
        ui.add_space(4.0);
        form::kv(ui, "name", |ui| {
            if face.own {
                let id = ui.id().with(("face-name", &face.file));
                let typed = input::drafted(ui, id, &face.name, |ui, draft| {
                    InputField::new(draft).show(ui)
                });
                let wanted = typed.map(|typed| typed.trim().to_owned());
                if let Some(wanted) = wanted
                    && !wanted.is_empty()
                    && wanted != face.name
                {
                    rename_to = Some(wanted);
                }
            } else {
                ui.label(theme::mono(&face.name));
                ui.label(theme::faint("one of the six every bust has"));
            }
        });
        form::kv(ui, "file", |ui| {
            ui.label(theme::mono(face.part()).color(theme::CODE).size(12.5));
            ui.label(theme::faint("drop a PNG onto its card to replace it").size(12.0));
        });
        form::kv_text(ui, "size", size_text(&face.name, own_size, &sizes));
    });
    let own_lines: Vec<&FaceLine> = lines
        .iter()
        .filter(|line| faces.iter().any(|f| f.own && f.name == line.face))
        .collect();
    let title = faces::speaker_name(editor, key);
    form::section_count(
        ui,
        &format!("Lines with {title}'s own faces"),
        own_lines.len(),
    );
    let shown = 6;
    let mut open = None;
    for line in own_lines.iter().take(shown) {
        ui.horizontal(|ui| {
            widgets::badge(ui, ScriptDef::TABLE);
            if ui
                .link(theme::mono(shown_path(&line.script)).size(12.5))
                .clicked()
            {
                open = Some(line.script.clone());
            }
            ui.label(theme::muted("›").size(12.0));
            ui.label(theme::mono(&line.node).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::faint(&line.face).size(12.0));
            });
        });
    }
    if own_lines.len() > shown {
        ui.label(theme::faint(format!("… {} more", own_lines.len() - shown)));
    }
    if own_lines.is_empty() {
        form::hint(ui, "no line asks for an own face yet");
    }
    if let Some(script) = open {
        editor.open_row(ScriptDef::TABLE, &script);
    }
    if let Some(wanted) = rename_to
        && faces::rename(editor, key, face, &wanted)
    {
        let view = editor.views.bust.view_mut(key);
        view.face = Some(wanted);
    }
    if remove_with.is_some() {
        remove_dialog(editor, key, face, faces, uses_of_face);
    }
}

fn remove_dialog(editor: &mut Editor, key: &str, face: &BustFace, faces: &[BustFace], uses: usize) {
    let view = editor.views.bust.view_mut(key);
    view.removing = Some(super::FaceRemoval {
        face: face.clone(),
        replacement: faces
            .iter()
            .find(|f| f.name != face.name)
            .map(|f| f.name.clone())
            .unwrap_or_default(),
        uses,
    });
}

pub fn remove_face_dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(removing) = editor
        .views
        .bust
        .views
        .get(key)
        .and_then(|view| view.removing.clone())
    else {
        return;
    };
    let faces = match editor.read::<BustDef>(key) {
        Some(Ok(bust)) => faces::faces(&bust),
        _ => Vec::new(),
    };
    let mut decision = None;
    let mut replacement = removing.replacement.clone();
    dialog::card(ctx, "remove-face", 400.0, |ui| {
        ui.label(
            RichText::new(format!("Remove face {}", removing.face.name))
                .size(16.0)
                .strong(),
        );
        ui.add_space(6.0);
        if removing.uses > 0 {
            ui.label(format!(
                "{} lines ask for it. They take another face instead:",
                removing.uses
            ));
            controls::combo("face-replacement")
                .selected_text(replacement.clone())
                .show_ui(ui, |ui| {
                    for other in faces.iter().filter(|f| f.name != removing.face.name) {
                        ui.selectable_value(&mut replacement, other.name.clone(), &other.name);
                    }
                });
        } else {
            ui.label(theme::muted("No line asks for it."));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if controls::danger(ui, "Remove").clicked() {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
        if ui.input(|i| EditorKey::Cancel.pressed(i)) {
            decision = Some(false);
        }
    });
    {
        let view = editor.views.bust.view_mut(key);
        if let Some(removing) = view.removing.as_mut() {
            removing.replacement = replacement.clone();
        }
    }
    let Some(apply) = decision else {
        return;
    };
    let view = editor.views.bust.view_mut(key);
    view.removing = None;
    if apply && faces::remove(editor, key, &removing.face, &replacement) {
        let view = editor.views.bust.view_mut(key);
        view.face = None;
        editor.select(RowSelection::row(BustDef::TABLE, key));
    }
}

fn size_text(
    face: &str,
    own: Option<[usize; 2]>,
    sizes: &[(String, Option<[usize; 2]>)],
) -> String {
    let Some(size) = own else {
        return "missing file".to_owned();
    };
    let others: Vec<(&str, [usize; 2])> = sizes
        .iter()
        .filter(|(name, _)| name != face)
        .filter_map(|(name, other)| Some((name.as_str(), (*other)?)))
        .collect();
    let differing = others.iter().find(|(_, other)| *other != size);
    match differing {
        _ if others.is_empty() => format!("{} × {}", size[0], size[1]),
        None => format!("{} × {} · same as the others", size[0], size[1]),
        Some((name, other)) => format!(
            "{} × {} · differs from {name} ({} × {}); every face of one bust is one size",
            size[0], size[1], other[0], other[1]
        ),
    }
}
