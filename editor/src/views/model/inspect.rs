//! What the loader makes of the model: the parts it draws for its build, how heavy they draw
//! together, its hitbox, and who wears it.

use engine::core::content::media::GltfInfos;
use engine::systems::actor::appearance;
use engine::systems::actor::model::{MAX_CHARACTER_TRIANGLES, ModelDef};
use ui::egui::Ui;
use ui::{form, panel, theme};

use crate::app::Editor;
use crate::tabs::shown_path;

/// A part of the model by its path, with the file of its mesh for the model's build and how many
/// triangles it draws, when it draws on the build.
type DrawnPart = (String, Option<(String, usize)>);

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelDef) {
    let drawn: Option<Vec<DrawnPart>> = super::built(editor, key).map(|(content, _)| {
        let infos = content.built::<GltfInfos>();
        appearance::laid(content, row.laid_parts())
            .parts
            .into_iter()
            .map(|laid| {
                let part = laid.part;
                let mesh = part.get(content).mesh_for(row.build.as_ref()).map(|mesh| {
                    let triangles = infos.0.get(mesh.as_str()).map_or(0, |info| info.triangles);
                    (mesh.as_str().to_owned(), triangles)
                });
                (part.path(content).to_owned(), mesh)
            })
            .collect()
    });
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Body", "", |_| {});
        ui.add_space(4.0);
        form::kv_text(
            ui,
            "build",
            row.build.as_ref().map_or("any", |build| build.as_str()),
        );
        match &drawn {
            Some(parts) => {
                for (part, mesh) in parts {
                    form::kv(ui, shown_path(part), |ui| match mesh {
                        Some((file, triangles)) => {
                            ui.label(theme::mono(file).size(12.5));
                            ui.label(theme::faint(format!("{triangles} triangles")).size(12.0));
                        }
                        None => {
                            ui.label(theme::faint("no mesh for this build: not drawn"));
                        }
                    });
                }
                let triangles: usize = parts
                    .iter()
                    .filter_map(|(_, mesh)| mesh.as_ref().map(|(_, triangles)| *triangles))
                    .sum();
                form::kv(ui, "triangles", |ui| {
                    let ink = if triangles > MAX_CHARACTER_TRIANGLES {
                        theme::DANGER
                    } else {
                        theme::TEXT
                    };
                    ui.colored_label(ink, format!("{triangles} of {MAX_CHARACTER_TRIANGLES}"));
                });
            }
            None => {
                ui.colored_label(
                    theme::DANGER,
                    "the model did not load: its problems are in the output panel",
                );
            }
        }
        form::kv_text(
            ui,
            "hitbox",
            format!(
                "{:.2} m across · {:.2} m tall",
                row.hitbox.radius.0 * 2.0,
                row.hitbox.height.0
            ),
        );
    });
    ui.add_space(8.0);
    crate::shell::row_form::usages::show_usages_grouped(editor, ui, key, true);
}
