//! Bringing files from anywhere on disk into a folder of the game as they come: folders whole,
//! archives unpacked, FBX files converted to glTF binaries. They are read in the background, then
//! listed to pick from before anything is written: a file already in the game is replaced only
//! when picked, and one the game would read as content or settings is never imported.

mod fbx;
mod gather;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use engine::core::content::files::{GameFile, classify};
use engine::systems::registry;
use ui::egui::{self, Context, Ui};
use ui::{controls, dialog, form, theme, words};

pub use fbx::glb_of_fbx;
pub use gather::{Came, Failure, Found, Gathered, gather};

use crate::app::{Editor, ModalDialog};
use crate::project::{ContentEdit, FileBytes, ProjectFiles, files};
use crate::shell::content_tree;

pub struct ImportDialog {
    pub folder: String,
    reading: Option<Reading>,
    pub files: Vec<Incoming>,
    pub failed: Vec<Failure>,
    search: String,
    error: Option<String>,
}

/// A file found to import, by its path in the game.
pub struct Incoming {
    pub path: String,
    pub bytes: Vec<u8>,
    pub came: Came,
    pub take: bool,
    pub replaces: bool,
    pub refused: Option<String>,
}

struct Reading {
    now: Arc<Mutex<(usize, String)>>,
    done: JoinHandle<Gathered>,
}

/// Reads `sources` in the background and opens the list of what they hold to import into `folder`.
pub fn begin(editor: &mut Editor, folder: String, sources: Vec<PathBuf>) {
    if editor.project.is_none() {
        editor.toast("Open a game folder to import files into it");
        return;
    }
    let now = Arc::new(Mutex::new((0, String::new())));
    let told = now.clone();
    let done = std::thread::spawn(move || {
        gather(&sources, &|path| {
            if let Ok(mut now) = told.lock() {
                now.0 += 1;
                now.1 = path.to_owned();
            }
        })
    });
    editor.dialogs.push(ModalDialog::Import(ImportDialog {
        folder,
        reading: Some(Reading { now, done }),
        files: Vec::new(),
        failed: Vec::new(),
        search: String::new(),
        error: None,
    }));
}

/// What `gathered` would bring into `folder` of `files`: every file picked but those already there,
/// and those refused.
pub fn review(
    files: &ProjectFiles,
    folder: &str,
    gathered: Gathered,
) -> (Vec<Incoming>, Vec<Failure>) {
    let types = registry().file_types();
    let incoming = gathered
        .files
        .into_iter()
        .map(|found| {
            let path = if folder.is_empty() {
                found.path
            } else {
                format!("{folder}/{}", found.path)
            };
            let refused = refusal(&path, &types);
            let replaces = files.contains(&path);
            Incoming {
                take: refused.is_none() && !replaces,
                path,
                bytes: found.bytes,
                came: found.came,
                replaces,
                refused,
            }
        })
        .collect();
    (incoming, gathered.failed)
}

/// Writes every picked file of `incoming` into the game as one step. Gives their paths.
pub fn import(editor: &mut Editor, incoming: Vec<Incoming>) -> Result<Vec<String>, String> {
    let taken: Vec<Incoming> = incoming
        .into_iter()
        .filter(|file| file.take && file.refused.is_none())
        .collect();
    if taken.is_empty() {
        return Err("no file is picked to import".to_owned());
    }
    let mut edit = ContentEdit::new(match &taken[..] {
        [one] => format!("Import {}", crate::tabs::shown_path(&one.path)),
        many => format!("Import {}", words::counted(many.len(), "file")),
    });
    let mut made = Vec::with_capacity(taken.len());
    for file in taken {
        made.push(file.path.clone());
        edit.write(file.path, FileBytes::from(file.bytes));
    }
    editor.try_apply(edit)?;
    Ok(made)
}

/// Imports what `sources` hold into `folder` at once, as the list offers them: everything but what
/// is already there or refused. Gives the paths written and what could not be read.
pub fn import_now(
    editor: &mut Editor,
    folder: &str,
    sources: &[PathBuf],
) -> Result<(Vec<String>, Vec<Failure>), String> {
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let (incoming, failed) = review(&project.files, folder, gather(sources, &|_| {}));
    Ok((import(editor, incoming)?, failed))
}

pub fn dialog(editor: &mut Editor, ctx: &Context, mut dialog: ImportDialog) -> Option<ModalDialog> {
    match dialog.reading.take() {
        Some(reading) if reading.done.is_finished() => match reading.done.join() {
            Ok(gathered) => {
                let project = editor.project.as_ref()?;
                (dialog.files, dialog.failed) = review(&project.files, &dialog.folder, gathered);
            }
            Err(_) => dialog.error = Some("reading the files stopped short".to_owned()),
        },
        Some(reading) => {
            dialog.reading = Some(reading);
            ctx.request_repaint();
        }
        None => {}
    }
    let mut outcome = None;
    let mut pressed = dialog::Keys::default();
    dialog::card(ctx, "import", 760.0, |ui| {
        pressed = dialog::keys(ui);
        dialog::title(
            ui,
            &format!("Import into {}", content_tree::folder_title(&dialog.folder)),
        );
        ui.add_space(4.0);
        match &dialog.reading {
            Some(reading) => {
                let (count, now) = reading
                    .now
                    .lock()
                    .map(|now| now.clone())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading {} · {now}", words::counted(count, "file")));
                });
            }
            None => listing(ui, &mut dialog),
        }
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(8.0);
        let picked = dialog.files.iter().filter(|file| file.take).count();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(dialog.reading.is_none() && picked > 0, |ui| {
                if controls::primary(ui, format!("Import {}", words::counted(picked, "file")))
                    .clicked()
                {
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
        Some(true) => {
            let folder = dialog.folder.clone();
            match import(editor, std::mem::take(&mut dialog.files)) {
                Ok(made) => {
                    if let Some(first) = made.first() {
                        content_tree::reveal(editor, first);
                    }
                    editor.toast(format!(
                        "Imported {} into {}",
                        words::counted(made.len(), "file"),
                        content_tree::folder_title(&folder)
                    ));
                    None
                }
                Err(error) => {
                    editor.toast(error);
                    None
                }
            }
        }
        Some(false) => None,
        None => Some(ModalDialog::Import(dialog)),
    }
}

fn listing(ui: &mut Ui, dialog: &mut ImportDialog) {
    let count =
        |test: &dyn Fn(&Incoming) -> bool| dialog.files.iter().filter(|file| test(file)).count();
    let converted = count(&|file| matches!(file.came, Came::Converted { .. }));
    let unpacked = count(&|file| matches!(file.came, Came::Unpacked { .. }));
    let existing = count(&|file| file.replaces);
    let refused = count(&|file| file.refused.is_some());
    let mut said = vec![words::counted(dialog.files.len(), "file")];
    for (count, what) in [
        (converted, "converted from FBX"),
        (unpacked, "unpacked from archives"),
        (existing, "already in the game"),
        (refused, "refused"),
    ] {
        if count > 0 {
            said.push(format!("{count} {what}"));
        }
    }
    ui.label(theme::muted(said.join(" · ")));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut dialog.search)
                .hint_text("filter")
                .desired_width(220.0),
        );
        let search = dialog.search.to_lowercase();
        let shown =
            |file: &Incoming| file.refused.is_none() && file.path.to_lowercase().contains(&search);
        if ui.button("Pick all").clicked() {
            dialog
                .files
                .iter_mut()
                .filter(|file| shown(file))
                .for_each(|file| file.take = true);
        }
        if ui.button("Pick none").clicked() {
            dialog
                .files
                .iter_mut()
                .filter(|file| shown(file))
                .for_each(|file| file.take = false);
        }
        if existing > 0 && ui.button("Skip those already in the game").clicked() {
            dialog
                .files
                .iter_mut()
                .filter(|file| file.replaces)
                .for_each(|file| file.take = false);
        }
    });
    let search = dialog.search.to_lowercase();
    let shown: Vec<usize> = (0..dialog.files.len())
        .filter(|&index| dialog.files[index].path.to_lowercase().contains(&search))
        .collect();
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("import-files")
        .max_height(420.0)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, shown.len(), |ui, rows| {
            for &index in &shown[rows] {
                let file = &mut dialog.files[index];
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(file.refused.is_none(), |ui| {
                        ui.checkbox(&mut file.take, "");
                    });
                    let shown = file
                        .path
                        .strip_prefix(&dialog.folder)
                        .map_or(file.path.as_str(), |rest| rest.trim_start_matches('/'));
                    ui.label(shown);
                    match (&file.refused, &file.came) {
                        (Some(why), _) => {
                            ui.colored_label(theme::DANGER, why);
                        }
                        (None, Came::Converted { from }) => {
                            ui.label(theme::faint(format!("from {}", file_name(from))));
                        }
                        _ => {}
                    }
                    if file.replaces && file.refused.is_none() {
                        ui.label(theme::faint(if file.take {
                            "replaces the game's"
                        } else {
                            "kept as the game has it"
                        }));
                    }
                });
            }
        });
    if !dialog.failed.is_empty() {
        form::section_count(ui, "Could not be read", dialog.failed.len());
        egui::ScrollArea::vertical()
            .id_salt("import-failed")
            .max_height(120.0)
            .show(ui, |ui| {
                for failure in &dialog.failed {
                    ui.horizontal(|ui| {
                        ui.label(&failure.path);
                        ui.colored_label(theme::DANGER, &failure.why);
                    });
                }
            });
    }
}

/// Why the file at `path` is never imported: the game would read it as content or settings, or
/// refuse its name, or never see it.
fn refusal(path: &str, types: &[engine::core::content::files::FileType]) -> Option<String> {
    if path.split('/').any(|part| part.starts_with('.')) {
        return Some("hidden: the game never sees it".to_owned());
    }
    if files::content_holding(path).is_some() {
        return Some("named as content, which is made in the editor".to_owned());
    }
    match classify(path, false, types) {
        Err(problem) => Some(problem.to_string()),
        Ok(Some(GameFile::Settings)) => Some("the game's settings".to_owned()),
        _ => None,
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
