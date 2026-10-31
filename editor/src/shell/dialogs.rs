use std::path::{Path, PathBuf};
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::form;
use ui::input;
use ui::panel;
use ui::theme;
use ui::words;

use engine::core::content::{ContentFile, SETTINGS_FILE, fields_file, version};
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::player::PlayerDef;
use engine::systems::script::ScriptDef;
use strum::VariantArray;
use ui::egui::{Context, RichText, Ui};
use ui::glyph::{self, Mark};
use ui::sheet;

use super::row_form::settings;
use super::{content_tree, unused_dialog, widgets};
use crate::app::{
    CellsBenchDialog, CloseThen, ConfirmAction, Editor, ModalDialog, NewGameDialog, NewGameSource,
    PickRowThen,
};
use crate::icons;
use crate::keymap::{EditorKey, KeyScope};
use crate::play;
use crate::project::{ContentEdit, files};
use crate::rows::{self, UsageDecision};
use crate::shell::{self, start};
use crate::tabs::shown_path;
use crate::templates::{self, GameTemplate};
use crate::views::area::{pick, placement, resize};
use crate::views::script;
use crate::views::world;
use ui::input::InputField;

pub fn show(editor: &mut Editor, ctx: &Context) {
    let mut dialogs = std::mem::take(&mut editor.dialogs);
    let shown = dialogs.len();
    let mut keep = Vec::new();
    for dialog in dialogs.drain(..) {
        if let Some(dialog) = one(editor, ctx, dialog) {
            keep.push(dialog);
        }
    }
    let opened = std::mem::take(&mut editor.dialogs);
    // What a dialog did on closing, or one opened, shows only from the next frame.
    if keep.len() != shown || !opened.is_empty() {
        ctx.request_repaint();
    }
    keep.extend(opened);
    editor.dialogs = keep;
}

fn cells_bench(
    editor: &mut Editor,
    ctx: &Context,
    mut dialog: CellsBenchDialog,
) -> Option<ModalDialog> {
    let (lines, exit) = dialog.run.poll();
    for line in lines {
        if let Some(row) = line.text.strip_prefix(CELLS_BENCH_TAG) {
            match row
                .strip_prefix("RESULT ")
                .and_then(|result| result.split(' ').next())
                .and_then(|size| size.parse::<f32>().ok())
            {
                Some(size) => dialog.result = Some(Meters(size)),
                None => dialog.rows.push(row.to_owned()),
            }
        }
        editor.output.say(line.tag, line.text);
    }
    dialog.exit = exit;
    let mut outcome = None;
    let mut pressed = dialog::Keys::default();
    dialog::card(ctx, "cells-bench", 640.0, |ui| {
        pressed = dialog::keys(ui);
        dialog::title(ui, "Measuring the cell size");
        ui.label(theme::muted(
            "The spawn area grown four by four, stepped as one area on one core, at each size the cells could have.",
        ));
        ui.add_space(6.0);
        for row in &dialog.rows {
            ui.monospace(row);
        }
        match (dialog.exit, dialog.result) {
            (None, _) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(theme::muted("Measuring; a few minutes."));
                });
            }
            (Some(_), Some(size)) => {
                ui.label(format!("The cell size for this game: {} m.", size.0));
            }
            (Some(code), None) => {
                ui.label(theme::muted(format!(
                    "rift exited with code {code} without a result; its output is in the output panel."
                )));
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| match (dialog.exit, dialog.result) {
            (None, _) => {
                if ui.button("Cancel").clicked() {
                    outcome = Some(None);
                }
            }
            (Some(_), Some(size)) => {
                if controls::primary(ui, format!("Apply {} m", size.0)).clicked() {
                    outcome = Some(Some(size));
                }
                if ui.button("Discard").clicked() {
                    outcome = Some(None);
                }
            }
            (Some(_), None) => {
                if ui.button("Close").clicked() {
                    outcome = Some(None);
                }
            }
        });
    });
    if pressed.cancel {
        outcome = Some(None);
    }
    match outcome {
        Some(Some(size)) => {
            settings::set_cell_size(editor, size);
            None
        }
        Some(None) => {
            dialog.run.cancel();
            None
        }
        None => {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            Some(ModalDialog::CellsBench(dialog))
        }
    }
}

const CELLS_BENCH_TAG: &str = "[cells-bench] ";

fn new_game(editor: &mut Editor, ctx: &Context, mut dialog: NewGameDialog) -> Option<ModalDialog> {
    let mut outcome = None;
    let templates = templates::list();
    let mut pressed = dialog::Keys::default();
    let copy_source = match &dialog.source {
        NewGameSource::Copy(folder) => Some(folder.clone()),
        NewGameSource::Template(_) => None,
    };
    let title = match &copy_source {
        Some(folder) => format!("+ New game from {}", start::game_name(folder)),
        None => "+ New game".to_owned(),
    };
    dialog::card(
        ctx,
        "new-game",
        if copy_source.is_some() { 600.0 } else { 1080.0 },
        |ui| {
            pressed = dialog::keys(ui);
            dialog::title(ui, &title);
            match &mut dialog.source {
                NewGameSource::Template(picked) => {
                    ui.horizontal_top(|ui| {
                        template_cards(ui, picked, templates);
                        ui.add_space(20.0);
                        if let Some(template) = templates.iter().find(|t| t.key == *picked) {
                            template_summary(ui, editor, template);
                        }
                    });
                }
                NewGameSource::Copy(source) => {
                    form::kv_code(ui, "from", shell::pretty_path(source));
                }
            }
            ui.add_space(10.0);
            ui.separator();
            form::kv(ui, "name", |ui| {
                let response = InputField::new(&mut dialog.name).width(360.0).show(ui);
                if response.changed() && !dialog.folder_touched {
                    let base = Path::new(&dialog.folder)
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_default();
                    dialog.folder = base.join(start::slug(&dialog.name)).display().to_string();
                }
                ui.label(theme::muted(format!(
                    "written into the new {SETTINGS_FILE}"
                )));
            });
            form::kv(ui, "folder", |ui| {
                if InputField::new(&mut dialog.folder)
                    .width(300.0)
                    .show(ui)
                    .changed()
                {
                    dialog.folder_touched = true;
                }
                if ui.button("pick…").clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title("Folder for the new game (new or empty)")
                        .pick_folder()
                {
                    dialog.folder = folder.display().to_string();
                    dialog.folder_touched = true;
                }
                ui.label(theme::muted("new, or empty"));
            });
            if let Some(source) = &copy_source {
                ui.add_space(6.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(300.0);
                        sheet::section(ui, "Copied");
                        let files = files::content_files(source).unwrap_or_default();
                        ui.label(theme::mono(format!(
                            "{SETTINGS_FILE}   name → {}",
                            dialog.name
                        )));
                        ui.label(theme::mono(format!(
                            "every other file · {}",
                            files.len() - 1
                        )));
                    });
                    ui.vertical(|ui| {
                        sheet::section(ui, "Stays behind");
                        ui.label(theme::muted(
                            "files and folders whose names start with a dot",
                        ));
                    });
                });
                ui.add_space(6.0);
                ui.label(theme::muted(format!(
                    "{} isn't touched. Nothing links the two afterwards.",
                    start::game_name(source)
                )));
            } else {
                ui.add_space(6.0);
                let name = templates
                    .iter()
                    .find(|t| matches!(&dialog.source, NewGameSource::Template(k) if *k == t.key))
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                ui.label(theme::muted(format!(
                    "Create writes {name}'s files into {} and opens it. Nothing links the game to its template.",
                    shell::pretty_path(Path::new(&dialog.folder))
                )));
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(theme::DANGER, error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = if copy_source.is_some() {
                    "Create and open"
                } else {
                    "Create"
                };
                if controls::primary(ui, label).clicked() {
                    outcome = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    outcome = Some(false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        theme::faint(format!(
                            "{} picks · {} creates · {} cancels",
                            EditorKey::Move.name(),
                            EditorKey::Confirm.name(),
                            EditorKey::Cancel.name()
                        ))
                        .size(12.0),
                    );
                });
            });
        },
    );
    if pressed.confirm && !ctx.egui_wants_keyboard_input() {
        outcome = Some(true);
    }
    if pressed.cancel {
        outcome = Some(false);
    }
    match outcome {
        Some(true) => {
            let folder = PathBuf::from(dialog.folder.trim());
            let result = match &dialog.source {
                NewGameSource::Template(template) => {
                    templates::new_game(template, dialog.name.trim(), &folder)
                }
                NewGameSource::Copy(source) => {
                    templates::new_game_from(source, dialog.name.trim(), &folder)
                }
            };
            match result {
                Ok(()) => {
                    editor.say(format!("Created {}", folder.display()));
                    shell::request_close(editor, CloseThen::Open(folder));
                    None
                }
                Err(error) => {
                    dialog.error = Some(error);
                    Some(ModalDialog::NewGame(dialog))
                }
            }
        }
        Some(false) => None,
        None => Some(ModalDialog::NewGame(dialog)),
    }
}

fn template_cards(ui: &mut Ui, picked: &mut String, templates: &[GameTemplate]) {
    if !ui.ctx().egui_wants_keyboard_input()
        && let Some([_, step]) = ui.input(|i| EditorKey::Move.arrow(i))
        && let Some(at) = templates.iter().position(|t| t.key == *picked)
    {
        let next = at
            .saturating_add_signed(step as isize)
            .min(templates.len() - 1);
        *picked = templates[next].key.clone();
    }
    ui.vertical(|ui| {
        ui.set_width(460.0);
        sheet::section(ui, "Built into this editor");
        for template in templates {
            let selected = *picked == template.key;
            let response = panel::card(selected)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(theme::strong(&template.name));
                                ui.label(theme::muted(&template.key).monospace());
                            });
                            ui.label(theme::muted(template_counts(template)));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                theme::faint(format!("build {}", version::build()))
                                    .monospace()
                                    .size(12.0),
                            );
                        });
                    });
                })
                .response
                .interact(egui::Sense::click());
            if response.clicked() {
                *picked = template.key.clone();
            }
        }
    });
}

fn template_counts(template: &GameTemplate) -> String {
    let areas = template.rows_in(AreaDef::TABLE);
    let players = template.rows_in(PlayerDef::TABLE);
    let scripts = template.rows_in(ScriptDef::TABLE);
    format!(
        "{areas} area{} · {players} player{} · {scripts} script{}",
        words::plural(areas),
        words::plural(players),
        words::plural(scripts)
    )
}

fn template_summary(ui: &mut Ui, editor: &Editor, template: &GameTemplate) {
    ui.vertical(|ui| {
        ui.set_width(560.0);
        form::kv(ui, "name", |ui| {
            ui.label(theme::strong(&template.name));
            ui.label(theme::muted(format!("from {SETTINGS_FILE}")));
        });
        form::kv_code(ui, "initial spawn", &template.initial_spawn);
        form::kv(ui, "holds", |ui| {
            let mut parts: Vec<String> = vec![SETTINGS_FILE.to_owned(), "every fixture".to_owned()];
            for (table, keys) in &template.rows {
                let own: Vec<&str> = keys
                    .iter()
                    .map(String::as_str)
                    .filter(|key| !editor.shapes.is_fixture(table, key))
                    .collect();
                if !own.is_empty() {
                    parts.push(
                        own.iter()
                            .map(|key| shown_path(key))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
            }
            ui.add(egui::Label::new(theme::muted(parts.join(" · "))).wrap());
        });
    });
}

fn one(editor: &mut Editor, ctx: &Context, dialog: ModalDialog) -> Option<ModalDialog> {
    match dialog {
        ModalDialog::Confirm(confirm) => {
            let mut outcome = None;
            let mut pressed = dialog::Keys::default();
            dialog::card(
                ctx,
                "confirm",
                if confirm.compare.is_some() {
                    600.0
                } else {
                    440.0
                },
                |ui| {
                    pressed = dialog::keys(ui);
                    dialog::title(ui, &confirm.title);
                    ui.label(theme::muted(&confirm.body));
                    if let Some(compare) = &confirm.compare {
                        ui.add_space(8.0);
                        ui.columns(2, |columns| {
                            sheet::section(&mut columns[0], &compare.left_title);
                            panel::code_block(&mut columns[0], &compare.left);
                            sheet::section(&mut columns[1], &compare.right_title);
                            panel::code_block(&mut columns[1], &compare.right);
                        });
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let clicked = if confirm.action.destroys() {
                            controls::danger(ui, &confirm.label).clicked()
                        } else {
                            controls::primary(ui, &confirm.label).clicked()
                        };
                        if clicked {
                            outcome = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Some(false);
                        }
                    });
                },
            );
            if pressed.confirm {
                outcome = Some(true);
            }
            if pressed.cancel {
                outcome = Some(false);
            }
            match outcome {
                Some(true) => {
                    confirmed(editor, confirm.action);
                    None
                }
                Some(false) => None,
                None => Some(ModalDialog::Confirm(confirm)),
            }
        }
        ModalDialog::Close(close) => {
            let mut outcome = None;
            dialog::window(ctx, "Unsaved changes", |ui| {
                let count = editor.project.as_ref().map_or(0, |p| p.unsaved().len());
                ui.label(format!("{count} files are unsaved."));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        outcome = Some(Some(true));
                    }
                    if ui.button("Don't save").clicked() {
                        outcome = Some(Some(false));
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(None);
                    }
                });
            });
            match outcome {
                Some(Some(true)) => {
                    if editor.save() {
                        shell::finish_close(editor, close.then);
                    }
                    None
                }
                Some(Some(false)) => {
                    shell::finish_close(editor, close.then);
                    None
                }
                Some(None) => None,
                None => Some(ModalDialog::Close(close)),
            }
        }
        ModalDialog::NewGame(dialog) => new_game(editor, ctx, dialog),
        ModalDialog::Conflicts(mut conflicts) => {
            let conflict = conflicts.first().cloned()?;
            let name = conflict.file.clone();
            let disk_text = conflict
                .disk
                .as_ref()
                .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok());
            let mine_text = editor
                .project
                .as_ref()
                .and_then(|project| project.files.text(&conflict.file));
            let (disk_lines, mine_lines) =
                differing_lines(disk_text.as_deref(), mine_text.as_deref());
            let compare_id = egui::Id::new(("conflict-compare", &conflict.file));
            let mut compare = ctx
                .data(|d| d.get_temp::<bool>(compare_id))
                .unwrap_or(false);
            let mut decided: Option<bool> = None;
            dialog::card(
                ctx,
                "conflict",
                if compare { 1100.0 } else { 560.0 },
                |ui| {
                    dialog::title(ui, &format!("{name} changed on disk"));
                    ui.label(
                        theme::muted(
                            "You have unsaved edits to this row, and the file on disk changed too.",
                        )
                        .size(13.5),
                    );
                    ui.add_space(8.0);
                    if compare {
                        ui.columns(2, |columns| {
                            for (column, title, text) in [
                                (0usize, "on disk", disk_text.as_deref()),
                                (1, "yours", mine_text.as_deref()),
                            ] {
                                let ui = &mut columns[column];
                                ui.label(theme::faint(title).monospace().size(12.0));
                                egui::ScrollArea::vertical()
                                    .id_salt(("compare", column))
                                    .max_height(420.0)
                                    .show(ui, |ui| {
                                        panel::code_block(ui, text.unwrap_or("(the file is gone)"));
                                    });
                            }
                        });
                    } else {
                        panel::well().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for (title, lines) in [("on disk", &disk_lines), ("yours", &mine_lines)]
                            {
                                ui.label(theme::faint(title).monospace().size(12.5));
                                for line in lines {
                                    ui.label(
                                        RichText::new(format!("    {}", line.trim()))
                                            .monospace()
                                            .size(12.5)
                                            .color(theme::TEXT),
                                    );
                                }
                                if lines.is_empty() {
                                    ui.label(theme::faint("    (nothing)").monospace().size(12.5));
                                }
                            }
                        });
                    }
                    if conflicts.len() > 1 {
                        ui.add_space(4.0);
                        ui.label(
                            theme::faint(format!(
                                "{} more rows wait behind this one",
                                conflicts.len() - 1
                            ))
                            .size(12.0),
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if controls::primary(ui, "Keep mine")
                            .on_hover_text("The next save wins")
                            .clicked()
                        {
                            decided = Some(false);
                        }
                        if ui
                            .button("Take disk")
                            .on_hover_text("The edits go")
                            .clicked()
                        {
                            decided = Some(true);
                        }
                        if ui
                            .button(if compare {
                                "Back to the summary"
                            } else {
                                "Open both side by side"
                            })
                            .clicked()
                        {
                            compare = !compare;
                            ctx.data_mut(|d| d.insert_temp(compare_id, compare));
                        }
                    });
                },
            );
            let Some(take) = decided else {
                return Some(ModalDialog::Conflicts(conflicts));
            };
            ctx.data_mut(|d| d.remove_temp::<bool>(compare_id));
            conflicts.remove(0);
            if take && let Some(project) = &mut editor.project {
                project.take_disk(&conflict);
            }
            (!conflicts.is_empty()).then_some(ModalDialog::Conflicts(conflicts))
        }
        ModalDialog::Delete(mut dialog) => {
            let mut outcome = None;
            let mut pressed = dialog::Keys::default();
            dialog::card(ctx, "delete", 620.0, |ui| {
                pressed = dialog::keys(ui);
                ui.horizontal(|ui| {
                    glyph::show(ui, Mark::Cross, 16.0, theme::DANGER);
                    dialog::title(ui, &rows::delete_title(&dialog));
                });
                ui.add_space(4.0);
                rows::delete_body(editor, ui, &mut dialog);
                ui.add_space(8.0);
                let edits: usize = dialog.groups.iter().map(|group| group.usages.len()).sum();
                let ready = dialog.groups.iter().all(|group| match &group.decision {
                    UsageDecision::Undecided => false,
                    UsageDecision::Repoint(key) => !key.is_empty(),
                    UsageDecision::Remove | UsageDecision::Unpack => true,
                });
                ui.horizontal(|ui| {
                    let unpacked: usize = dialog
                        .groups
                        .iter()
                        .filter(|group| group.decision == UsageDecision::Unpack)
                        .map(|group| group.usages.len())
                        .sum();
                    let deleted = dialog.gone.len();
                    let label = if edits == 0 && deleted <= 1 {
                        "Delete".to_owned()
                    } else if edits == 0 {
                        format!("Delete {deleted} files")
                    } else if unpacked == edits {
                        format!(
                            "Delete and unpack {edits} placement{}",
                            words::plural(edits)
                        )
                    } else {
                        format!("Delete and apply {edits} edit{}", words::plural(edits))
                    };
                    ui.add_enabled_ui(ready, |ui| {
                        if controls::danger(ui, &label).clicked() {
                            outcome = Some(true);
                        }
                    });
                    if ui.button("Cancel").clicked() {
                        outcome = Some(false);
                    }
                });
            });
            if pressed.cancel {
                outcome = Some(false);
            }
            match outcome {
                Some(true) => match rows::delete(editor, &dialog) {
                    Ok(()) => None,
                    Err(error) => {
                        dialog.error = Some(error);
                        Some(ModalDialog::Delete(dialog))
                    }
                },
                Some(false) => None,
                None => Some(ModalDialog::Delete(dialog)),
            }
        }
        ModalDialog::Import(dialog) => crate::import::dialog(editor, ctx, dialog),
        ModalDialog::FindUnused(dialog) => unused_dialog::find_unused_dialog(editor, ctx, dialog),
        ModalDialog::CellsBench(dialog) => cells_bench(editor, ctx, dialog),
        ModalDialog::Binaries => {
            let mut open = true;
            dialog::card(ctx, "binaries", 560.0, |ui| {
                if play::binaries_body(editor, ui) || dialog::keys(ui).cancel {
                    open = false;
                }
            });
            open.then_some(ModalDialog::Binaries)
        }
        ModalDialog::Keys => {
            let mut open = true;
            let mut pressed = dialog::Keys::default();
            dialog::card(ctx, "keys", 1200.0, |ui| {
                pressed = dialog::keys(ui);
                ui.horizontal(|ui| {
                    dialog::title(ui, "Keys");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Done").clicked() {
                            open = false;
                        }
                    });
                });
                ui.add_space(8.0);
                let columns = key_sheet_columns();
                let tallest = ctx.content_rect().height() * 0.75;
                egui::ScrollArea::vertical()
                    .min_scrolled_height(tallest)
                    .max_height(tallest)
                    .show(ui, |ui| {
                        ui.columns(columns.len(), |uis| {
                            for (ui, scopes) in uis.iter_mut().zip(&columns) {
                                for scope in scopes {
                                    sheet::section(ui, scope.title());
                                    egui::Grid::new(("keys", scope.title()))
                                        .num_columns(2)
                                        .spacing([14.0, 4.0])
                                        .show(ui, |ui| {
                                            for key in scope.keys() {
                                                controls::key_hint(ui, &key.name());
                                                ui.label(RichText::new(key.about()).size(13.0));
                                                ui.end_row();
                                            }
                                        });
                                    ui.add_space(10.0);
                                }
                            }
                        });
                    });
            });
            (open && !pressed.cancel).then_some(ModalDialog::Keys)
        }
        ModalDialog::Resize(dialog) => {
            resize::resize_dialog(editor, ctx, dialog).map(ModalDialog::Resize)
        }
        ModalDialog::Arrange(dialog) => {
            world::arrange_dialog(editor, ctx, dialog).map(ModalDialog::Arrange)
        }
        ModalDialog::RenameNode(dialog) => {
            script::rename_node_dialog(editor, ctx, dialog).map(ModalDialog::RenameNode)
        }
        ModalDialog::LinkNode(dialog) => {
            script::link_node_dialog(editor, ctx, dialog).map(ModalDialog::LinkNode)
        }
        ModalDialog::PickRow(mut dialog) => {
            let mut outcome = None;
            dialog::window(ctx, &dialog.title, |ui| {
                let mut candidates: Vec<String> = if dialog.options.is_empty() {
                    editor
                        .project
                        .as_ref()
                        .map(|p| p.files.paths_of(&dialog.table))
                        .unwrap_or_default()
                } else {
                    dialog.options.clone()
                };
                let editing = dialog.then.editing();
                candidates.sort_by_cached_key(|key| {
                    (crate::forms::seen_from(editing, key).0, key.clone())
                });
                let search = input::search(ui, &mut dialog.search, "search", None);
                input::default_focus(ui, &search);
                let needle = dialog.search.to_lowercase();
                let listed: Vec<(String, String, Option<String>)> = candidates
                    .into_iter()
                    .filter(|key| needle.is_empty() || key.to_lowercase().contains(&needle))
                    .map(|key| {
                        let shown = match (key.as_str(), editing) {
                            ("", _) => "the game folder".to_owned(),
                            (key, Some(_)) => crate::forms::seen_from(editing, key).1,
                            (key, None) => key.to_owned(),
                        };
                        let refused = editing
                            .filter(|_| dialog.table == BlueprintDef::TABLE)
                            .zip(editor.project.as_ref())
                            .and_then(|(into, project)| {
                                placement::holds_itself(project, into, &key)
                            });
                        let shown = shown_path(&shown).to_owned();
                        (key, shown, refused)
                    })
                    .collect();
                let pickable: Vec<&str> = listed
                    .iter()
                    .filter(|(_, _, refused)| refused.is_none())
                    .map(|(key, ..)| key.as_str())
                    .collect();
                let at = dialog
                    .picked
                    .as_deref()
                    .and_then(|picked| pickable.iter().position(|key| *key == picked));
                let step = ui.input(|i| EditorKey::Move.arrow(i)).map(|[_, step]| step);
                let moved_to = match (at, step) {
                    (Some(at), Some(step)) => Some(
                        at.saturating_add_signed(step as isize)
                            .min(pickable.len() - 1),
                    ),
                    (None, _) if !pickable.is_empty() => Some(0),
                    _ => None,
                };
                if let Some(to) = moved_to {
                    dialog.picked = Some(pickable[to].to_owned());
                }
                let pressed = dialog::keys(ui);
                if pressed.confirm && dialog.picked.is_some() {
                    outcome = Some(true);
                }
                if pressed.cancel {
                    outcome = Some(false);
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (key, shown, refused) in &listed {
                            let kind = match &dialog.then {
                                PickRowThen::NewFile { .. } => icons::of_table(key),
                                PickRowThen::MoveInto { .. } | PickRowThen::MoveNodes { .. } => {
                                    Mark::Folder.into()
                                }
                                PickRowThen::AddModule { .. }
                                | PickRowThen::ReplaceBlueprint { .. } => {
                                    icons::of_table(&dialog.table)
                                }
                            };
                            if let Some(reason) = refused {
                                ui.add_enabled_ui(false, |ui| {
                                    widgets::kind_option(
                                        ui,
                                        kind,
                                        &format!("{shown} · {reason}"),
                                        false,
                                    )
                                });
                                continue;
                            }
                            let picked = dialog.picked.as_deref() == Some(key.as_str());
                            let response = widgets::kind_option(ui, kind, shown, picked)
                                .on_hover_text(shown_path(key));
                            if picked && step.is_some() {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                dialog.picked = Some(key.clone());
                            }
                            if response.double_clicked() {
                                dialog.picked = Some(key.clone());
                                outcome = Some(true);
                            }
                        }
                        if listed.is_empty() {
                            ui.label(theme::muted("nothing matches"));
                        }
                    });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(dialog.picked.is_some(), egui::Button::new("Pick"))
                        .clicked()
                    {
                        outcome = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(false);
                    }
                });
            });
            match outcome {
                Some(true) => {
                    let picked = dialog.picked.clone().unwrap_or_default();
                    match dialog.then {
                        PickRowThen::NewFile { folder } => {
                            content_tree::new_file(editor, &folder, &picked)
                        }
                        PickRowThen::MoveNodes {
                            bundle,
                            ids,
                            folders,
                        } => crate::views::area::panel::move_into_picked(
                            editor, &bundle, &ids, &folders, &picked,
                        ),
                        PickRowThen::MoveInto { paths } => {
                            if let Err(error) = rows::move_into(editor, &paths, &picked) {
                                editor.toast(error);
                            }
                        }
                        PickRowThen::AddModule { area, kind, at } => {
                            pick::add_picked(editor, &area, kind, at, &picked)
                        }
                        PickRowThen::ReplaceBlueprint { area, node } => {
                            placement::replace_with(editor, &area, &node, &picked)
                        }
                    }
                    None
                }
                Some(false) => None,
                None => Some(ModalDialog::PickRow(dialog)),
            }
        }
    }
}

fn confirmed(editor: &mut Editor, action: ConfirmAction) {
    match action {
        ConfirmAction::ReplaceModule {
            table,
            key,
            index,
            module,
        } => {
            let (Some(value), Some(reader)) = (editor.fields_json(&key), editor.reader()) else {
                return;
            };
            let replaced = reader.modules_in(&table, &value).and_then(|mut modules| {
                *modules.get_mut(index).ok_or("the module is gone")? = module;
                reader.with_modules_in(&table, &value, &modules)
            });
            match replaced {
                Ok(value) => {
                    let mut edit =
                        ContentEdit::new(format!("Replace module {index} of {}", shown_path(&key)));
                    edit.write_json(fields_file(&key), &value);
                    editor.apply(edit);
                }
                Err(error) => editor.toast(error),
            }
        }
        ConfirmAction::DeleteScriptNodes { script, names } => {
            script::delete_nodes_now(editor, &script, names)
        }
    }
}

fn differing_lines(a: Option<&str>, b: Option<&str>) -> (Vec<String>, Vec<String>) {
    let a: Vec<&str> = a.unwrap_or("").lines().collect();
    let b: Vec<&str> = b.unwrap_or("").lines().collect();
    let only_in = |from: &[&str], other: &[&str]| -> Vec<String> {
        from.iter()
            .filter(|line| !other.contains(line))
            .take(6)
            .map(|line| (*line).to_owned())
            .collect()
    };
    (only_in(&a, &b), only_in(&b, &a))
}

fn key_sheet_columns() -> Vec<Vec<KeyScope>> {
    let mut columns: Vec<(usize, Vec<KeyScope>)> = vec![(0, Vec::new()); KEY_SHEET_COLUMNS];
    for scope in KeyScope::VARIANTS {
        let shortest = columns
            .iter_mut()
            .min_by_key(|(rows, _)| *rows)
            .expect("a column");
        shortest.0 += scope.keys().count() + 1;
        shortest.1.push(*scope);
    }
    columns.into_iter().map(|(_, scopes)| scopes).collect()
}

const KEY_SHEET_COLUMNS: usize = 3;
