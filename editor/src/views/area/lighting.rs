//! The lighting page of an area's view: the light the area stands in, part by part, beside the
//! area lit by it under its first spell of weather.

use engine::core::content::ContentFile;
use engine::systems::area::AreaDef;
use ui::egui::Ui;
use ui::{form, panel, sheet, theme};

use super::nodes::SceneBundle;
use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::row_form;
use std::sync::Arc;

pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let bundle = editor.views.area.view(view_key).bundle.clone();
    let Some(Ok(area)) = editor.read::<AreaDef>(&bundle) else {
        ui.label(theme::muted("This area doesn't load; Output says why.").size(12.5));
        return;
    };
    let mut lighting = area.lighting.clone();
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        let mut cx = FormCx::schemaless(project, shapes, form, &mut actions)
            .editing(AreaDef::TABLE, Some(&bundle));
        part(
            ui,
            "Sun",
            "Where the sun stands at noon and its colour; the full moon stands opposite it by \
             night. Without one, the area is lit by its ambient light alone.",
            |ui| changed |= forms::of_type(ui, &mut cx, &mut lighting.sun, "/lighting/sun"),
        );
        part(
            ui,
            "Sky",
            "The air the sun lights, or the dark of a cave or a stage, which no sun lights.",
            |ui| changed |= forms::of_type(ui, &mut cx, &mut lighting.sky, "/lighting/sky"),
        );
        part(
            ui,
            "Dome",
            "How the dome of an open sky is painted, with the weather's clouds, the moon and the \
             stars.",
            |ui| changed |= forms::of_type(ui, &mut cx, &mut lighting.dome, "/lighting/dome"),
        );
        part(
            ui,
            "Ambient",
            "The light everywhere, in every shadow.",
            |ui| changed |= forms::of_type(ui, &mut cx, &mut lighting.ambient, "/lighting/ambient"),
        );
        part(
            ui,
            "Grade",
            "How the picture is finished: its tint, exposure, saturation, contrast and warmth.",
            |ui| changed |= forms::of_type(ui, &mut cx, &mut lighting.grade, "/lighting/grade"),
        );
        part(
            ui,
            "Air",
            "How many times more the water in saturated air scatters the sky's light than dry \
             air's dust.",
            |ui| {
                changed |= forms::of_type(
                    ui,
                    &mut cx,
                    &mut lighting.moist_scatter,
                    "/lighting/moist_scatter",
                )
            },
        );
    }
    for action in actions {
        row_form::act(editor, action, AreaDef::TABLE, &bundle);
    }
    if let Some(label) = changed.label(&bundle) {
        let mut edited = (*area).clone();
        edited.lighting = lighting;
        editor.write_fields(&label, &bundle, SceneBundle::Area(Arc::new(edited)));
    }
}

/// A part of the lighting under its `title`, what it is, and its fields as `edit` draws them.
fn part(ui: &mut Ui, title: &str, about: &str, edit: impl FnOnce(&mut Ui)) {
    sheet::section(ui, title);
    panel::tile_across(ui, false, |ui| {
        form::hint(ui, about);
        edit(ui);
    });
    ui.add_space(8.0);
}
