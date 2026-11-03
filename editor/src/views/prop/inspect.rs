use engine::core::content::FileSize;
use engine::core::content::media::GltfInfos;
use engine::systems::area::prop::{
    Footprint, MAX_PROP_BYTES, MAX_PROP_MATERIALS, MAX_PROP_TRIANGLES, PropDef,
};
use engine::systems::movement::Bodies;
use ui::egui::{self, Ui, Vec2};
use ui::{form, panel, theme};

use super::footprint::FootprintKind;
use crate::app::Editor;
use crate::row_form::usages;
use crate::rows;
use crate::thumbs;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef) {
    egui::ScrollArea::vertical()
        .id_salt(("prop-inspect", key))
        .show(ui, |ui| {
            model_card(editor, ui, key, row);
            ui.add_space(8.0);
            footprint_card(editor, ui, key, row);
            ui.add_space(8.0);
            usages::show_usages_grouped(editor, ui, key, true);
        });
}

fn model_card(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef) {
    let path = row.model.path();
    let info = editor.project.as_ref().and_then(|project| {
        project
            .content
            .as_ref()?
            .built::<GltfInfos>()
            .0
            .get(&path)
            .cloned()
    });
    let weight = editor
        .project
        .as_ref()
        .and_then(|project| project.files.get(&path))
        .map(|bytes| FileSize(bytes.len()));
    let flat_shaded = super::geometry(editor, key).map(|geometry| geometry.flat_shaded);
    panel::tile_across(ui, false, |ui| {
        panel::header(ui, "Model", "", |ui| {
            if ui.button("Replace model…").clicked() {
                browse(editor, key);
                ui.close();
            }
        });
        ui.add_space(4.0);
        let side = Vec2::splat(thumbs::props::SIZE as f32);
        match editor.form.prop_thumbnails.thumbnail(key) {
            Some(texture) => {
                ui.image(egui::load::SizedTexture::new(texture, side));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(side, egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, theme::INPUT);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "drawing…",
                    egui::FontId::proportional(12.0),
                    theme::MUTED,
                );
                ui.ctx().request_repaint();
            }
        }
        ui.add_space(4.0);
        form::kv_code(ui, "file", path);
        match info {
            Some(info) => {
                let size = info.bounds.max - info.bounds.min;
                form::kv_text(
                    ui,
                    "size",
                    format!("{:.2} × {:.2} × {:.2} m", size.x, size.z, size.y),
                );
                budget(ui, "triangles", info.triangles, MAX_PROP_TRIANGLES);
                budget(ui, "materials", info.materials.len(), MAX_PROP_MATERIALS);
                if let Some(weight) = weight {
                    form::kv(ui, "weight", |ui| {
                        let ink = if weight > MAX_PROP_BYTES {
                            theme::DANGER
                        } else {
                            theme::TEXT
                        };
                        ui.colored_label(ink, format!("{weight} of {MAX_PROP_BYTES}"));
                    });
                }
                form::kv(ui, "shading", |ui| match flat_shaded {
                    Some(true) => {
                        ui.label("flat");
                    }
                    Some(false) => {
                        ui.label("smooth");
                    }
                    None => {
                        ui.label(theme::muted("unread"));
                    }
                });
            }
            None => {
                ui.colored_label(theme::DANGER, "the model does not read as a glTF binary");
            }
        }
        ui.add_space(4.0);
        form::hint(ui, "Drop a .glb on this tab to replace the model.");
    });
}

fn footprint_card(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef) {
    let current = editor
        .project
        .as_ref()
        .and_then(|project| project.files.hash_of(&row.model.path()));
    let bodies = editor
        .project
        .as_ref()
        .and_then(|project| Bodies::of(project.content.as_ref()?));
    let mut refit = false;
    panel::tile_across(ui, false, |ui| {
        panel::subtitled(ui, "Footprint", FootprintKind::of(&row.footprint).label());
        ui.add_space(4.0);
        if let Footprint::Polygon { points, fitted_to } = &row.footprint {
            form::kv_text(ui, "corners", points.len().to_string());
            form::kv(ui, "fitted", |ui| match fitted_to {
                Some(hash) if Some(&hash.0) == current.as_ref() => {
                    ui.colored_label(theme::OK, "to this model");
                }
                Some(_) => {
                    ui.colored_label(theme::DANGER, "to an older model");
                    refit = ui.button("Refit").clicked();
                }
                None => {
                    ui.label(theme::muted("shaped by hand"));
                }
            });
        }
        if let Some(bodies) = bodies {
            form::hint(
                ui,
                &format!(
                    "Walkers keep {} m off the footprint, the dashed line round it.",
                    bodies.radius.0
                ),
            );
        }
    });
    if refit {
        super::fit_to_mesh(editor, key);
    }
}

fn budget(ui: &mut Ui, key: &str, count: usize, most: usize) {
    form::kv(ui, key, |ui| {
        let ink = if count > most {
            theme::DANGER
        } else {
            theme::TEXT
        };
        ui.colored_label(ink, format!("{count} of {most}"));
    });
}

fn browse(editor: &mut Editor, key: &str) {
    let Some(picked) = rows::picked_files(
        rfd::FileDialog::new()
            .set_title("Replace model")
            .add_filter("glTF binary", &["glb"]),
    ) else {
        return;
    };
    let Some(path) = picked.first() else {
        return;
    };
    match std::fs::read(path) {
        Ok(bytes) => {
            super::replace_model(editor, key, bytes);
        }
        Err(error) => editor.toast(format!("{}: {error}", path.display())),
    }
}
