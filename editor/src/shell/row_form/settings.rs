use engine::core::content::SETTINGS_FILE;
use engine::core::units::Meters;
use engine::systems::mirror;
use ui::egui::Ui;
use ui::{sheet, theme};

use crate::app::{CellsBenchDialog, Editor, ModalDialog, PlacePick};
use crate::forms::{self, FormAction, FormCx};
use crate::play::CellsRun;
use crate::project::ContentEdit;

pub fn show_game(editor: &mut Editor, ui: &mut Ui) {
    let Some(mut value) = editor
        .project
        .as_ref()
        .and_then(|project| project.files.json_owned(SETTINGS_FILE))
    else {
        return;
    };
    ui.label(theme::muted(SETTINGS_FILE).monospace().size(13.0));
    ui.add_space(4.0);
    let shapes = editor.shapes.clone();
    let mut actions = Vec::new();
    let changed = {
        let Editor {
            project,
            form,
            shapes: known,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::new(project, &shapes.settings, known, form, &mut actions);
        ui.push_id("game", |ui| {
            forms::object_without(
                ui,
                &mut cx,
                &shapes.settings,
                &mut value,
                "",
                &["content_version"],
            )
        })
        .inner
    };
    if changed.is_some() {
        let mut edit = ContentEdit::new("Edit the game");
        edit.write_json(SETTINGS_FILE, &value);
        editor.apply(edit);
    }
    cells_guidance(editor, ui);
    for action in actions {
        match action {
            FormAction::OpenRow { table, key } => {
                editor.open_row(&table, &key);
            }
            FormAction::PickPlace { path } => {
                editor.pick = Some(PlacePick {
                    file: SETTINGS_FILE.to_owned(),
                    pointer: path,
                    area: None,
                });
                editor.toast("Click a walkable cell on an area to put the initial spawn there");
            }
            FormAction::OpenNode { .. }
            | FormAction::NewRow { .. }
            | FormAction::RowAction(_)
            | FormAction::Reveal { .. }
            | FormAction::NewScriptModule
            | FormAction::NewObjectScript { .. } => {}
        }
    }
}

/// Sets the game's cell size.
pub fn set_cell_size(editor: &mut Editor, size: Meters) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Ok(settings) = project.settings() else {
        editor.toast("The settings don't read as they stand");
        return;
    };
    let mut settings = (*settings).clone();
    settings.spatial_partition_cell_size = size;
    let mut edit = ContentEdit::new("Set the cell size");
    if let Err(problem) = edit.write_settings(project, &settings) {
        editor.toast(problem);
        return;
    }
    editor.apply(edit);
}

/// Where the cell size comes from: the bench measures it, on the content as saved.
fn cells_guidance(editor: &mut Editor, ui: &mut Ui) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.as_ref() else {
        return;
    };
    let (halo, reached_by) = mirror::halo(content);
    ui.add_space(8.0);
    sheet::section(ui, "Cells");
    ui.label(theme::muted(format!(
        "Worlds see {} m past their cell, as far as {reached_by}; a cell is at least that.",
        halo.0
    )));
    ui.label(theme::muted(
        "The size is this game's own tuning: measured as the spawn area grown four by four, stepped on one core at each size the cells could have, on the game as saved.",
    ));
    if ui.button("Measure the cell size…").clicked() {
        if !editor.save() {
            return;
        }
        match CellsRun::start(editor) {
            Ok(run) => editor
                .dialogs
                .push(ModalDialog::CellsBench(CellsBenchDialog {
                    run,
                    rows: Vec::new(),
                    result: None,
                    exit: None,
                })),
            Err(problem) => editor.toast(problem),
        }
    }
}
