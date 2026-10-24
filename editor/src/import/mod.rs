pub mod formats;
pub mod records;
pub mod tileset;
mod tileset_ui;

pub use records::{ImportRecord, ImportRecords};
pub use tileset_ui::{
    TilesetReimport, inspect_reimport as inspect_tileset_reimport,
    show_reimport as show_tileset_reimport,
};

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use ui::dialog;
use ui::theme;

use engine::core::content::{ContentFile, fields_file};
use engine::systems::area::tileset::TilesetDef;
use formats::{AnimationExport, DroppedFiles};
use image::RgbaImage;
use serde::Deserialize;
use ui::egui::{Context, Ui};
use ui::glyph::{self, Mark};

use crate::app::{ChannelInbox, Editor, ModalDialog};
use crate::rows;
use crate::shell::active_tab;
use crate::tabs::EditorTab;
use crate::tabs::shown_path;

pub const IMPORT_PART: &str = "/import";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStep {
    Files(Vec<PathBuf>),
    Apply,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportTarget {
    Tileset(String),
    NewTileset,
    Animation(String),
}

pub struct ImportReading {
    pub id: u64,
    pub target: ImportTarget,
    pub dropped: DroppedFiles,
    pub again: Option<records::DueReimport>,
    receiver: ChannelInbox<Result<AnimationExport, String>>,
}

pub enum ImportDialog {
    Tileset(Box<tileset_ui::TilesetDialog>),
    Animation(Box<tileset_ui::AnimationDialog>),
}

#[derive(Default)]
pub struct ImportJobs {
    pub reading: Vec<ImportReading>,
    pub script: VecDeque<ImportStep>,
    next_id: u64,
}

pub fn dialog(editor: &mut Editor, ctx: &Context, dialog: ImportDialog) -> Option<ImportDialog> {
    match dialog {
        ImportDialog::Tileset(dialog) => tileset_ui::dialog(editor, ctx, *dialog)
            .map(|next| ImportDialog::Tileset(Box::new(next))),
        ImportDialog::Animation(dialog) => tileset_ui::animation_dialog(editor, ctx, *dialog)
            .map(|next| ImportDialog::Animation(Box::new(next))),
    }
}

pub fn show_readings(editor: &mut Editor, ctx: &Context) {
    let mut cancelled = Vec::new();
    for job in editor.jobs.reading.iter().filter(|job| job.again.is_none()) {
        let name = names_of(job);
        let mut cancel = false;
        dialog::card(ctx, &format!("import-reading-{}", job.id), 360.0, |ui| {
            ui.label(theme::strong(format!("Reading {name}…")));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::muted("decoding the export off the interface thread"));
            });
            ui.add_space(10.0);
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if cancel {
            cancelled.push(job.id);
        }
    }
    editor
        .jobs
        .reading
        .retain(|job| !cancelled.contains(&job.id));
}

pub fn poll(editor: &mut Editor) {
    if editor.project.is_none() {
        return;
    }
    for due in editor.imports.poll() {
        start_again(editor, due);
    }
    let mut done = Vec::new();
    for (index, job) in editor.jobs.reading.iter().enumerate() {
        if let Ok(result) = job.receiver.try_recv() {
            done.push((index, result));
        }
    }
    for (index, result) in done.into_iter().rev() {
        let job = editor.jobs.reading.remove(index);
        match result {
            Ok(export) => land(editor, job, export),
            Err(error) => {
                if let Some(due) = job.again.clone() {
                    if !editor.imports.retry(due) {
                        editor.toast(format!("Could not read {}: {error}", names_of(&job)));
                    }
                } else {
                    editor.toast(format!("Not imported: {error}"));
                }
            }
        }
    }
}

pub fn tick(editor: &mut Editor, ctx: &Context) {
    let dropped: Vec<PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .collect()
    });
    if !dropped.is_empty() {
        if matches!(active_tab(editor), Some(EditorTab::Model { .. })) {
            editor.views.model.drops = dropped;
        } else {
            self::dropped(editor, dropped);
        }
    }
    scripted(editor);
}

fn scripted(editor: &mut Editor) {
    let Some(step) = editor.jobs.script.front().cloned() else {
        return;
    };
    if !editor.jobs.reading.is_empty() {
        return;
    }
    match step {
        ImportStep::Files(paths) => {
            editor.jobs.script.pop_front();
            let files = match content_parent(editor) {
                Some(parent) => paths.iter().map(|path| parent.join(path)).collect(),
                None => paths,
            };
            dropped(editor, files);
        }
        ImportStep::Apply => {
            let Some(index) = editor
                .dialogs
                .iter()
                .position(|dialog| matches!(dialog, ModalDialog::Import(_)))
            else {
                match active_tab(editor) {
                    Some(EditorTab::Tileset { key })
                        if editor.views.tileset.reimports.contains_key(&key) =>
                    {
                        editor.jobs.script.pop_front();
                        tileset_ui::apply_reimport(editor, &key);
                    }
                    _ => {}
                }
                return;
            };
            let ModalDialog::Import(dialog) = editor.dialogs.remove(index) else {
                return;
            };
            editor.jobs.script.pop_front();
            let kept = match *dialog {
                ImportDialog::Tileset(dialog) => tileset_ui::apply_now(editor, *dialog)
                    .map(|d| ImportDialog::Tileset(Box::new(d))),
                ImportDialog::Animation(dialog) => tileset_ui::apply_animation_now(editor, *dialog)
                    .map(|d| ImportDialog::Animation(Box::new(d))),
            };
            if let Some(kept) = kept {
                editor
                    .dialogs
                    .insert(index, ModalDialog::Import(Box::new(kept)));
            }
        }
    }
}

pub fn dropped(editor: &mut Editor, files: Vec<PathBuf>) {
    if editor.project.is_none() {
        editor.toast("Open a content folder before importing");
        return;
    }
    let dropped = match formats::sort_out(&files) {
        Ok(dropped) => dropped,
        Err(reason) => {
            editor.toast(format!("Not imported: {reason}"));
            return;
        }
    };
    let target = match active_tab(editor) {
        Some(EditorTab::Tileset { key }) => match dropped {
            DroppedFiles::Png(_) => ImportTarget::Tileset(key),
            _ => ImportTarget::Animation(key),
        },
        _ => match dropped {
            DroppedFiles::Png(_) => ImportTarget::NewTileset,
            _ => {
                editor.toast("Not imported: an animation export goes onto an open tileset");
                return;
            }
        },
    };
    begin(editor, target, dropped, None);
}

pub fn start_for_tileset(editor: &mut Editor, key: &str) {
    pick_files(editor, ImportTarget::Tileset(key.to_owned()));
}

pub fn start_animation(editor: &mut Editor, key: &str) {
    pick_files(editor, ImportTarget::Animation(key.to_owned()));
}

fn pick_files(editor: &mut Editor, target: ImportTarget) {
    let title = match &target {
        ImportTarget::Tileset(key) => format!("Import onto {}", shown_path(key)),
        ImportTarget::NewTileset => "Import a tileset".to_owned(),
        ImportTarget::Animation(key) => format!("Import an animation onto {}", shown_path(key)),
    };
    let mut picker = rfd::FileDialog::new()
        .set_title(&title)
        .add_filter("exports", &["png", "json", "gif"]);
    if let Some(dir) = art_dir(editor) {
        picker = picker.set_directory(dir);
    }
    let Some(files) = picked_files(picker) else {
        return;
    };
    match formats::sort_out(&files) {
        Ok(dropped) => begin(editor, target, dropped, None),
        Err(reason) => editor.toast(format!("Not imported: {reason}")),
    }
}

pub fn picked_files(dialog: rfd::FileDialog) -> Option<Vec<PathBuf>> {
    // The desktop portal answers a cancelled pick with no files rather than with none.
    dialog.pick_files().filter(|files| !files.is_empty())
}

pub fn art_dir(editor: &Editor) -> Option<PathBuf> {
    let art = content_parent(editor)?.join("art");
    art.is_dir().then_some(art)
}

pub fn show_path(editor: &Editor, file: &Path) -> String {
    content_parent(editor)
        .and_then(|parent| file.strip_prefix(parent).ok())
        .map(|rest| rest.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| formats::name(file))
}

fn begin(
    editor: &mut Editor,
    target: ImportTarget,
    dropped: DroppedFiles,
    again: Option<records::DueReimport>,
) {
    let (sender, receiver) = channel();
    let job_dropped = dropped.clone();
    std::thread::spawn(move || {
        let _ = sender.send(formats::decode(&job_dropped));
    });
    editor.jobs.next_id += 1;
    editor.jobs.reading.push(ImportReading {
        id: editor.jobs.next_id,
        target,
        dropped,
        again,
        receiver: ChannelInbox::new(receiver),
    });
}

fn start_again(editor: &mut Editor, due: records::DueReimport) {
    let Some(record) = editor
        .imports
        .record_with(&due.table, &due.key, due.block)
        .cloned()
    else {
        return;
    };
    let Ok(dropped) = formats::sort_out(&record.files) else {
        editor.toast(format!(
            "{} changed but no longer reads as an export",
            formats::export_name(&record.files)
        ));
        return;
    };
    let target = match (due.table.as_str(), due.block) {
        (TilesetDef::TABLE, Some(_)) => ImportTarget::Animation(due.key.clone()),
        (TilesetDef::TABLE, None) => ImportTarget::Tileset(due.key.clone()),
        _ => return,
    };
    let present = editor
        .project
        .as_ref()
        .is_some_and(|project| project.files.contains(&fields_file(&due.key)));
    if !present {
        editor.imports.forget(&due.table, &due.key);
        return;
    }
    begin(editor, target, dropped, Some(due));
}

fn land(editor: &mut Editor, job: ImportReading, export: AnimationExport) {
    let next = match &job.target {
        ImportTarget::Tileset(key) => tileset_ui::land(editor, key, export, job.again.as_ref()),
        ImportTarget::NewTileset => tileset_ui::land_new(editor, export),
        ImportTarget::Animation(key) => {
            tileset_ui::land_animation(editor, key, export, job.again.as_ref())
        }
    };
    if let Some(next) = next {
        editor.dialogs.push(ModalDialog::Import(Box::new(next)));
    }
}

fn names_of(job: &ImportReading) -> String {
    formats::names(&job.dropped.files())
}

/// Whether `key` names a file of `Row` that is there but doesn't read, saying so: an import onto
/// it would take it for a new file and lose what it holds.
fn refuses_unread<Row: ContentFile>(editor: &mut Editor, key: &str) -> bool {
    let present = editor
        .project
        .as_ref()
        .is_some_and(|project| project.files.contains(&fields_file(key)));
    if !present || matches!(editor.read::<Row>(key), Some(Ok(_))) {
        return false;
    }
    editor.toast(format!(
        "Not imported onto {}: it doesn't read; its row on the left shows why",
        shown_path(key)
    ));
    true
}

fn asset_image(editor: &Editor, path: &str) -> Option<RgbaImage> {
    let bytes = editor.project.as_ref()?.files.get(path)?;
    formats::decode_image_bytes(bytes).ok()
}

/// The name a new file made from `export` starts with: the export's own, as a name may hold it,
/// or the type's.
fn suggested_name(export: &AnimationExport, table: &str) -> String {
    export
        .files
        .first()
        .map(|file| rows::name_from(&formats::stem(file).replace('-', "_")))
        .filter(|name| engine::core::content::valid_name(name))
        .unwrap_or_else(|| table.to_owned())
}

fn check_line(ui: &mut Ui, ok: bool, text: String) {
    ui.horizontal_wrapped(|ui| {
        if ok {
            glyph::show(ui, Mark::Check, 14.0, theme::OK);
        } else {
            glyph::show(ui, Mark::Warning, 14.0, theme::WARN);
        }
        ui.label(theme::muted(text).size(12.5));
    });
}

fn content_parent(editor: &Editor) -> Option<&Path> {
    editor.project.as_ref()?.root.parent()
}
