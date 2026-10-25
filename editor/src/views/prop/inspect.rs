use engine::core::assets::FileSize;
use engine::core::content::media::GltfInfos;
use engine::systems::area::prop::{
    Footprint, MAX_PROP_BYTES, MAX_PROP_MATERIALS, MAX_PROP_TRIANGLES, PropDef,
};
use engine::systems::movement::AGENT_RADIUS;
use ui::egui::{self, Ui, Vec2};
use ui::{form, panel, theme};

use super::footprint::Kind;
use crate::app::Editor;
use crate::rows;
use crate::shell::row_form::usages;
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
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
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
                budget(ui, "materials", info.materials, MAX_PROP_MATERIALS);
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
                        ui.colored_label(theme::OK, "flat");
                    }
                    Some(false) => {
                        ui.colored_label(theme::WARN, "smooth · the style wants flat faces");
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
        .and_then(|project| project.files.hash_of(&row.model.path()))
        .map(str::to_owned);
    let mut refit = false;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Footprint", Kind::of(&row.footprint).label(), |_| {});
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
        form::hint(
            ui,
            &format!(
                "Walkers keep {} m off the footprint, the dashed line round it.",
                AGENT_RADIUS.0
            ),
        );
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
