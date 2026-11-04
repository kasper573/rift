//! The terrain page of an area's view: the ground the area lies on, how far its heights reach
//! and how steep a slope still walks, and the terrains its paint lays, the first wherever nothing
//! is painted, beside the area they make.

use engine::core::content::ContentFile;
use engine::systems::area::{AreaDef, TerrainDef, Walk};
use ui::egui::{self, Color32, Ui};
use ui::{form, paint, panel, sheet, theme};

use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::{picker, row_form};

pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let bundle = editor.views.area.view(view_key).bundle.clone();
    let Some(Ok(area)) = editor.read::<AreaDef>(&bundle) else {
        ui.label(theme::muted("This area doesn't load; Output says why.").size(12.5));
        return;
    };
    let mut edited = (*area).clone();
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
        sheet::section(ui, "Ground");
        panel::tile_across(ui, false, |ui| {
            changed |= forms::of_type(ui, &mut cx, &mut edited.height_range, "/height_range");
            form::hint(
                ui,
                "How low and how high the ground reaches; changing it stretches the land with it.",
            );
            form::kv(ui, "walks up to", |ui| {
                changed |= forms::of_type(ui, &mut cx, &mut edited.max_slope, "/max_slope");
            });
            form::hint(ui, "Ground steeper than this slope is walked by nobody.");
        });
        ui.add_space(8.0);
        sheet::section(ui, "Terrains");
        panel::tile_across(ui, false, |ui| {
            for (index, terrain) in edited.terrains.iter_mut().enumerate() {
                let path = format!("/terrains/{index}");
                let def = project
                    .reader()
                    .path_of(*terrain)
                    .and_then(|key| project.read::<TerrainDef>(key).ok());
                form::kv(ui, if index == 0 { "ground" } else { "" }, |ui| {
                    swatch(ui, def.as_deref());
                    if picker::content(ui, &mut cx, terrain, &path) {
                        changed = FormChange::at(&path);
                    }
                });
            }
        });
        form::hint(
            ui,
            "The first terrain lies wherever nothing is painted. A terrain joins the list the \
             first time it is painted; picking another file here lays it wherever this one lies.",
        );
    }
    for action in actions {
        row_form::act(editor, action, AreaDef::TABLE, &bundle);
    }
    if let Some(label) = changed.label(&bundle) {
        editor.write_content(&label, &bundle, &edited);
    }
}

/// The colour a terrain is painted in on the map, hatched where nobody walks on it.
fn swatch(ui: &mut Ui, def: Option<&TerrainDef>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 14.0), egui::Sense::hover());
    let Some(def) = def else {
        ui.painter().rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.0, theme::MUTED),
            egui::StrokeKind::Inside,
        );
        return;
    };
    let [r, g, b, _] = def.color.0.to_be_bytes();
    ui.painter()
        .rect_filled(rect, 3.0, Color32::from_rgb(r, g, b));
    if def.walk == Walk::Blocks {
        paint::hatched(ui.painter(), rect, ui::ink::wash(theme::TEXT, 0.4), 8.0);
    }
}
