//! The fit page: the shown mesh on its skeleton's rig, idling, and every joint of the rig as the
//! mesh is skinned to it.

use engine::core::content::media::GltfInfos;
use engine::systems::actor::model::{fit_report, mesh_label};
use engine::systems::actor::part::ModelPartDef;
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
    Within(String),
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
        form::hint(
            ui,
            "No mesh yet: drop a .glb on this tab to give the part one.",
        );
        return;
    };
    let Some((content, skeleton)) = super::skeleton(editor, row) else {
        form::hint(
            ui,
            "The part's skeleton did not load, so nothing stands on the stage.",
        );
        return;
    };
    let skeleton = skeleton.clone();
    let skeleton_path = row.skeleton.path(content).to_owned();
    let rows = content
        .built::<GltfInfos>()
        .0
        .get(mesh.as_str())
        .map(|info| {
            let report = fit_report(&skeleton, &skeleton_path, &mesh_label(&mesh), info);
            let bound: Vec<&str> = info
                .skin_joints
                .iter()
                .map(|&joint| info.tree.nodes[joint].name.as_str())
                .collect();
            let rig = &skeleton.rig;
            rig.joints()
                .map(|joint| {
                    let name = rig.nodes[joint].name.clone();
                    let misfit = report
                        .fit
                        .misfits
                        .iter()
                        .position(|misfit| misfit.joint() == name);
                    let fits = match misfit {
                        Some(index) => Fits::Misfit(report.problems[index].found.clone()),
                        None if !bound.contains(&name.as_str()) => Fits::Unskinned,
                        None => {
                            let turn = info.tree.at_path(&rig.path(joint)).map_or(0.0, |found| {
                                rig.nodes[joint]
                                    .rest
                                    .rotation
                                    .angle_between(info.tree.nodes[found].rest.rotation)
                                    .to_degrees()
                            });
                            Fits::Within(format!("rests within {turn:.1}°"))
                        }
                    };
                    JointRow {
                        hand: skeleton
                            .def
                            .hands
                            .iter()
                            .find(|hand| hand.joint.0 == name)
                            .map(|hand| hand.name.clone()),
                        depth: rig.path(joint).len() - 1,
                        name,
                        fits,
                    }
                })
                .collect::<Vec<JointRow>>()
        });
    let stage_height = (ui.available_height() * 0.5).max(140.0);
    super::stand(
        editor,
        ui,
        key,
        Vec2::new(ui.available_width(), stage_height),
        &skeleton,
        mesh.path(),
    );
    ui.add_space(8.0);
    match rows {
        Some(rows) => {
            egui::ScrollArea::vertical()
                .id_salt(("part-fit", key))
                .auto_shrink([false, false])
                .show(ui, |ui| joints(ui, &rows));
        }
        None => form::hint(ui, "The mesh does not read as a glTF binary."),
    }
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
                    Fits::Within(note) => ui.label(theme::faint(note).size(12.0)),
                    Fits::Unskinned => ui.label(theme::faint("not skinned to it").size(12.0)),
                    Fits::Misfit(problem) => ui.colored_label(theme::DANGER, problem),
                };
                ui.end_row();
            }
        });
}
