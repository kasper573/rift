use engine::core::content::SETTINGS_FILE;
use engine::systems::GameSettings;
use ui::egui::Ui;
use ui::theme;

use crate::app::Editor;
use crate::day;
use crate::forms::{self, FormCx};
use crate::project::ContentEdit;

pub(crate) fn show_game(editor: &mut Editor, ui: &mut Ui) {
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
        if let Some(project) = editor.project.as_ref()
            && let Ok(before) = project.settings()
            && let Ok(after) = project.reader().from_json::<GameSettings>(&value)
        {
            let times = |settings: &GameSettings| {
                settings
                    .daylight
                    .as_ref()
                    .map_or_else(Vec::new, |cycle| cycle.times.clone())
            };
            let places = day::carry_names(project, &mut edit, &times(&before), &times(&after));
            if places > 0 {
                editor.form.status = vec![format!(
                    "The time of day was changed in {places} places that name it"
                )];
            }
        }
        editor.apply(edit);
    }
    for action in actions {
        super::act(editor, action, "", SETTINGS_FILE);
    }
}
