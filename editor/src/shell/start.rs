use std::path::{Path, PathBuf};
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::panel;
use ui::theme;
use ui::words;

use engine::core::content::{ContentScope, SETTINGS_FILE, version, with_content};
use engine::systems::GameSettings;
use ui::egui::{Align, Context, Layout, RichText, Ui};
use ui::glyph::Mark;

use super::report::{self, ReportAction};
use crate::app::{Editor, content_version};
use crate::keymap::EditorKey;
use crate::modal::{CloseThen, ModalDialog, NewGameDialog, NewGameSource};
use crate::project::files::content_of;
use crate::project::journal::RecoveryJournal;
use crate::shell;
use crate::widgets;

use crate::tabs::shown_path;
use crate::templates;
use ui::form;

pub fn show(editor: &mut Editor, root: &mut Ui) {
    let ctx = root.ctx().clone();
    let ctx = &ctx;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(60, 40)))
        .show(root, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(560.0);
                    ui.label(RichText::new("Open a game").strong().size(26.0));
                    ui.add_space(24.0);
                    recent_list(editor, ui);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if controls::primary(ui, "Open folder…").clicked() {
                            pick_folder(editor);
                        }
                        if ui.button("+ New game…").clicked() {
                            new_game_dialog(editor, None);
                        }
                    });
                    ui.add_space(24.0);
                    ui.label(
                        theme::faint(format!(
                            "build {} · content version {}",
                            version::build(),
                            content_version()
                        ))
                        .size(12.0),
                    );
                });
                ui.add_space(40.0);
                if let Some((folder, text)) = editor.start.report.clone() {
                    ui.vertical(|ui| {
                        match report::panel(ui, &super::pretty_path(&folder), &text, true) {
                            ReportAction::LoadAgain => editor.open_folder(&folder),
                            ReportAction::Close => {
                                editor.start.report = None;
                                editor.start.watch = None;
                            }
                            ReportAction::None => {}
                        }
                    });
                } else if let Some((folder, reason)) = editor.start.failed.clone() {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("{} did not open", super::pretty_path(&folder)))
                                .strong(),
                        );
                        ui.label(theme::muted(reason));
                        if ui.button("Close").clicked() {
                            editor.start.failed = None;
                        }
                    });
                }
            });
        });
    recover_dialog(editor, ctx);
}

fn recent_list(editor: &mut Editor, ui: &mut Ui) {
    let recent = editor.settings.recent.clone();
    if recent.is_empty() {
        ui.label(theme::muted(
            "None yet. Open a content folder, or start a new game.",
        ));
        return;
    }
    if editor
        .start
        .names
        .iter()
        .map(|(folder, _)| folder)
        .ne(recent.iter())
    {
        editor.start.names = recent
            .iter()
            .map(|folder| (folder.clone(), game_name(folder)))
            .collect();
    }
    let names = editor.start.names.clone();
    let mut picks = FolderPicks::default();
    for (folder, name) in &names {
        let path = super::pretty_path(folder);
        let ago = editor.settings.opened_ago(folder);
        let response = ui
            .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
                                    folder_menu(ui, folder, &mut picks);
                                });
                                ui.label(theme::faint(ago).monospace().size(12.0));
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.label(theme::strong(name));
                                    ui.add(
                                        egui::Label::new(theme::muted(path).monospace().size(12.5))
                                            .truncate(),
                                    );
                                });
                            });
                        });
                    });
            })
            .response;
        if response.hovered() {
            ui.painter()
                .rect_filled(response.rect, 0.0, ui::ink::gold_wash(0.2));
            ui.input_mut(|i| {
                if EditorKey::Confirm.consume(i) {
                    picks.open = Some(folder.clone());
                }
                if EditorKey::Duplicate.consume(i) {
                    picks.copy = Some(folder.clone());
                }
                if EditorKey::Delete.consume(i) {
                    picks.forget = Some(folder.clone());
                }
            });
        }
        if response.double_clicked() {
            picks.open = Some(folder.clone());
        }
        response.context_menu(|ui| folder_menu(ui, folder, &mut picks));
        form::rule(ui);
    }
    if let Some(folder) = picks.open {
        editor.open_folder(&folder);
    }
    if let Some(folder) = picks.copy {
        new_game_dialog(editor, Some(folder));
    }
    if let Some(folder) = picks.forget {
        editor.settings.forget(&folder);
        editor.save_settings();
    }
}

pub fn game_name(folder: &Path) -> String {
    std::fs::read_to_string(folder.join(SETTINGS_FILE))
        .ok()
        .and_then(|text| {
            // The folder isn't open, so its settings read without the paths their references
            // name; only the name is wanted.
            let (settings, _) = with_content(ContentScope::new(), || {
                serde_json::from_str::<GameSettings>(&text)
            });
            settings.ok().map(|settings| settings.name)
        })
        .unwrap_or_else(|| {
            folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

fn recover_dialog(editor: &mut Editor, ctx: &Context) {
    let Some((folder, data)) = editor.start.recover.take() else {
        return;
    };
    let mut keep = Some((folder.clone(), data));
    dialog::card(ctx, "recover", 520.0, |ui| {
        let (folder, data) = keep.as_ref().expect("kept");
        let when = ui::words::clock(data.written.0, false);
        dialog::title(ui, "Recover unsaved work?");
        ui.label(
            theme::muted(format!(
                "The editor closed at {when} with {} unsaved row{} of {}, kept in its journal outside the content folder.",
                data.files.len(),
                words::plural(data.files.len()),
                super::pretty_path(folder)
            ))
            .size(13.5),
        );
        ui.add_space(8.0);
        panel::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    egui::Grid::new("recovered files").show(ui, |ui| {
                        for file in &data.files {
                            let name = content_of(&file.path)
                                .map(|(_, key)| shown_path(key).to_owned())
                                .unwrap_or_else(|| file.path.clone());
                            ui.label(theme::mono(name).size(13.0));
                            ui.label(
                                theme::faint(if file.bytes.is_some() {
                                    "unsaved edits"
                                } else {
                                    "deleted here"
                                })
                                .monospace()
                                .size(12.0),
                            );
                            ui.end_row();
                        }
                    });
                });
        });
        let same_version = data.content_version == content_version();
        if !same_version {
            ui.add_space(6.0);
            ui.colored_label(
                theme::WARN,
                format!(
                    "Written for content version {}; this editor reads {}. They are set aside.",
                    data.content_version,
                    content_version()
                ),
            );
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let pressed = dialog::keys(ui);
            if same_version
                && (controls::primary(ui, "Recover as unsaved").clicked() || pressed.confirm)
            {
                let (folder, data) = keep.take().expect("kept");
                editor.open_now(&folder, data.files);
            }
            if keep.is_some() && (controls::danger(ui, "Discard").clicked() || pressed.cancel) {
                let (folder, _) = keep.take().expect("kept");
                let _ = RecoveryJournal::set_aside(&editor.user, &folder);
                editor.open_now(&folder, Vec::new());
            }
        });
    });
    if let Some(kept) = keep
        && editor.project.is_none()
    {
        editor.start.recover = Some(kept);
    }
}

pub(crate) fn pick_folder(editor: &mut Editor) {
    let start = editor
        .settings
        .recent
        .first()
        .and_then(|folder| folder.parent().map(Path::to_path_buf));
    let mut dialog = rfd::FileDialog::new().set_title("Open a content folder");
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    if let Some(folder) = dialog.pick_folder() {
        shell::request_close(editor, CloseThen::Open(folder));
    }
}

fn new_game_dialog(editor: &mut Editor, copy: Option<PathBuf>) {
    let base = editor
        .settings
        .recent
        .first()
        .and_then(|folder| folder.parent().map(Path::to_path_buf))
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let (source, name) = match copy {
        Some(folder) => {
            let name = format!("{} copy", game_name(&folder));
            (NewGameSource::Copy(folder), name)
        }
        None => {
            let key = templates::list()
                .first()
                .map(|template| template.key.clone())
                .unwrap_or_default();
            (NewGameSource::Template(key), "My game".to_owned())
        }
    };
    let folder = base.join(slug(&name)).display().to_string();
    editor.dialogs.push(ModalDialog::NewGame(NewGameDialog {
        source,
        name,
        folder,
        folder_touched: false,
        error: None,
    }));
}

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "game".to_owned()
    } else {
        out
    }
}

#[derive(Default)]
struct FolderPicks {
    open: Option<PathBuf>,
    copy: Option<PathBuf>,
    forget: Option<PathBuf>,
}

fn folder_menu(ui: &mut Ui, folder: &Path, picks: &mut FolderPicks) {
    if ui
        .add(widgets::shortcut_button("Open", EditorKey::Confirm))
        .clicked()
    {
        picks.open = Some(folder.to_path_buf());
        ui.close();
    }
    if ui
        .add(widgets::shortcut_button(
            "+ New game from this…",
            EditorKey::Duplicate,
        ))
        .clicked()
    {
        picks.copy = Some(folder.to_path_buf());
        ui.close();
    }
    if ui.button("Show folder").clicked() {
        let _ = opener::open(folder);
        ui.close();
    }
    ui.separator();
    if ui
        .add(widgets::shortcut_button(
            "Remove from list",
            EditorKey::Delete,
        ))
        .clicked()
    {
        picks.forget = Some(folder.to_path_buf());
        ui.close();
    }
}
