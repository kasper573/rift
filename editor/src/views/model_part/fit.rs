//! The fit page: the shown mesh on its skeleton's rig, idling, and every joint of the rig as the
//! mesh is skinned to it.

use engine::core::content::gltf::fit_skin;
use engine::core::content::name_of;
use engine::core::render::world::body::Skin;
use engine::systems::actor::part::ModelPartDef;
use engine::systems::actor::skeleton::{Colours, SkeletonDef, misfit_problem};
use ui::egui::{self, Ui, Vec2};
use ui::{form, theme};

use crate::app::Editor;

/// A joint of the rig as the mesh is skinned to it.
struct JointRow {
    name: String,
    depth: usize,
    hand: Option<String>,
    fits: Fits,
}

enum Fits {
    Within,
    Unskinned,
    Misfit(String),
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelPartDef) {
    let shown = editor.views.model_part.view(key).mesh.clone();
    let Some(mesh) = row
        .meshes()
        .find(|(build, _)| *build == shown.as_ref())
        .map(|(_, mesh)| mesh.clone())
    else {
        form::hint(ui, "No mesh yet: pick one by its path in the form.");
        return;
    };
    let rows = joint_rows(editor, row, mesh.as_str());
    let stage_height = (ui.available_height() * 0.5).max(140.0);
    match super::skeleton(editor, row) {
        Some((content, skeleton)) => {
            let skeleton = skeleton.clone();
            let skin = Skin {
                mesh: mesh.path(),
                paint: row.material.map(|material| {
                    material
                        .get(content)
                        .paint(&skeleton.def, &Colours::default())
                }),
            };
            super::stand(
                editor,
                ui,
                key,
                Vec2::new(ui.available_width(), stage_height),
                &skeleton,
                skin,
            );
        }
        None => form::hint(
            ui,
            "The game did not load, so nothing stands on the stage: its problems are in the output \
             panel.",
        ),
    }
    ui.add_space(8.0);
    match rows {
        Some(rows) => {
            egui::ScrollArea::vertical()
                .id_salt(("part-fit", key))
                .auto_shrink([false, false])
                .show(ui, |ui| joints(ui, &rows));
        }
        None => form::hint(
            ui,
            "The mesh does not read as a glTF binary, or the skeleton's rig does not stand.",
        ),
    }
}

/// Every joint of the rig of the part's skeleton, as the mesh at `mesh` is skinned to it, read
/// from the files as they stand so a misfit shows while it keeps the game from loading.
fn joint_rows(editor: &mut Editor, row: &ModelPartDef, mesh: &str) -> Option<Vec<JointRow>> {
    let skeleton_path = editor
        .reader()?
        .path_of::<SkeletonDef>(row.skeleton)?
        .to_owned();
    let def = editor.read::<SkeletonDef>(&skeleton_path)?.ok()?;
    let rig = crate::views::skeleton::rigging(editor, &skeleton_path, &def)?
        .rig
        .clone()?;
    let info = {
        let Editor { project, views, .. } = &mut *editor;
        views.skeleton.gltfs.info(&project.as_ref()?.files, mesh)?
    };
    let fit = fit_skin(&rig, &info);
    let rig_name = format!("the rig of {}", name_of(&skeleton_path));
    let bound: Vec<&str> = info
        .skin_joints
        .iter()
        .map(|&joint| info.tree.nodes[joint].name.as_str())
        .collect();
    Some(
        rig.joints()
            .map(|joint| {
                let name = rig.nodes[joint].name.clone();
                let fits = match fit.misfits.iter().find(|misfit| misfit.joint == name) {
                    Some(misfit) => Fits::Misfit(misfit_problem(mesh, &rig_name, misfit).found),
                    None if !bound.contains(&name.as_str()) => Fits::Unskinned,
                    None => Fits::Within,
                };
                JointRow {
                    hand: def
                        .hands
                        .iter()
                        .find(|hand| hand.joint.0 == name)
                        .map(|hand| hand.name.clone()),
                    depth: rig.path(joint).len() - 1,
                    name,
                    fits,
                }
            })
            .collect(),
    )
}

fn joints(ui: &mut Ui, rows: &[JointRow]) {
    let unfit = rows
        .iter()
        .filter(|row| matches!(row.fits, Fits::Misfit(_)))
        .count();
    form::section_count(ui, "Joints of the rig", rows.len());
    if unfit > 0 {
        ui.colored_label(
            theme::DANGER,
            format!("{unfit} joints do not fit: the loader refuses the part"),
        );
    }
    egui::Grid::new("fit-joints")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 2.0))
        .show(ui, |ui| {
            for row in rows {
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 12.0);
                    ui.label(theme::mono(&row.name).size(12.5));
                    if let Some(hand) = &row.hand {
                        ui.label(theme::faint(format!("hand · {hand}")).size(12.0));
                    }
                });
                match &row.fits {
                    Fits::Within => {
                        ui.label(theme::faint("rests where the skin binds it").size(12.0))
                    }
                    Fits::Unskinned => ui.label(theme::faint("not skinned to it").size(12.0)),
                    Fits::Misfit(problem) => ui.colored_label(theme::DANGER, problem),
                };
                ui.end_row();
            }
        });
}
