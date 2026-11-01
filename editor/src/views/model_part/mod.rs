//! The model part view: each mesh of the part standing on its skeleton's rig, and how it fits the
//! rig joint by joint. Its meshes are glTF files of the game folder, picked by path in its form.

pub mod fit;

use std::sync::Arc;

use engine::core::assets::FileSize;
use engine::core::content::media::GltfInfos;
use engine::core::content::{Content, ContentFile};
use engine::core::render::world::body::{Figure, Skin};
use engine::systems::actor::model::Skeletons;
use engine::systems::actor::part::{MAX_PART_BYTES, ModelPartDef};
use engine::systems::actor::rigging::BuiltSkeleton;
use engine::systems::actor::skeleton::BuildName;
use engine::systems::actor::{Action, Motion, Pose};
use ui::egui::{self, Ui, Vec2};
use ui::{chip, form, panel, theme};

use super::area::viewport::StageClip;
use super::stage;
use crate::app::Editor;
use crate::shell::row_form;
use crate::tabs::shown_path;

pub struct ModelPartView {
    /// The mesh shown: the one cut for this build, or with `None` the one every build wears.
    pub mesh: Option<BuildName>,
    pub stage: stage::StageState,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ModelPartDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key, &row);
    ui::panel::left_scrolled(ui, "model_part", |ui| {
        row_form::show_row(editor, ui, ModelPartDef::TABLE, key)
    });
    ui::panel::inspector(ui, editor, "model_part", |editor, ui| {
        inspect(editor, ui, key, &row);
    });
    egui::Frame::new()
        .inner_margin(crate::shell::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            mesh_bar(editor, ui, key, &row);
            ui.add_space(4.0);
            fit::show(editor, ui, key, &row);
        });
}

pub fn ensure_view(editor: &mut Editor, key: &str, row: &ModelPartDef) {
    if editor.views.model_part.contains(key) {
        return;
    }
    let mesh = match &row.mesh {
        Some(_) => None,
        None => row.builds.keys().next().cloned(),
    };
    editor.views.model_part.insert(
        key,
        ModelPartView {
            mesh,
            stage: stage::StageState::framing(1.3),
        },
    );
}

/// The skeleton the part at `key` is made on, as the game built it, when both loaded.
pub fn skeleton<'c>(
    editor: &'c Editor,
    row: &ModelPartDef,
) -> Option<(&'c Content, &'c Arc<BuiltSkeleton>)> {
    let content = editor.project.as_ref()?.content.as_ref()?;
    Some((content, Skeletons::loaded(content, row.skeleton)?))
}

/// The clip `skeleton` stands a body in while idle.
pub(crate) fn rig_idle_clip(skeleton: &BuiltSkeleton) -> Option<StageClip> {
    let (cell, _) = skeleton
        .def
        .named(None, None, &Motion::Action(Action::Idle), Pose::Idle)?;
    let played = skeleton.played(cell.nth(0).clip())?;
    Some(StageClip::played(skeleton, played))
}

fn mesh_bar(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelPartDef) {
    let shown = editor.views.model_part.view(key).mesh.clone();
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        for (build, _) in row.meshes() {
            let label = build.map_or("every build", BuildName::as_str);
            if chip::mode(ui, label, shown.as_ref() == build).clicked() {
                editor.views.model_part.view_mut(key).mesh = build.cloned();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            stage::toggle(ui, "turning", &mut editor.views.toggles.turning);
        });
    });
}

/// Stands `skin` on the rig of `skeleton` in the middle of a stage over `size`, in its idle clip.
pub(super) fn stand(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    size: Vec2,
    skeleton: &BuiltSkeleton,
    skin: Skin,
) {
    let mut state = std::mem::take(&mut editor.views.model_part.view_mut(key).stage);
    let turntable = stage::turntable(editor, ui, &format!("model_part:{key}"), size, &mut state);
    editor.views.model_part.view_mut(key).stage = state;
    let Some(turntable) = turntable else {
        return;
    };
    let body = Figure {
        rig: skeleton.rig.clone(),
        skins: vec![skin],
    };
    let standing = stage::standing(body, rig_idle_clip(skeleton), None, Vec::new(), Vec::new());
    turntable.stand(editor, ui, vec![standing]);
}

/// What the loader makes of the part's meshes: each one's weight and how it draws, and who wears
/// the part.
fn inspect(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ModelPartDef) {
    let infos = editor
        .project
        .as_ref()
        .and_then(|project| project.content.as_ref())
        .map(|content| content.built::<GltfInfos>());
    let meshes: Vec<(String, Option<usize>, Option<FileSize>)> = row
        .meshes()
        .map(|(_, mesh)| {
            let triangles = infos
                .and_then(|infos| infos.0.get(mesh.as_str()))
                .map(|info| info.triangles);
            let weight = editor
                .project
                .as_ref()
                .and_then(|project| project.files.get(&mesh.path()))
                .map(|bytes| FileSize(bytes.len()));
            (mesh.as_str().to_owned(), triangles, weight)
        })
        .collect();
    panel::tile(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, "Meshes");
        ui.add_space(4.0);
        for (file, triangles, weight) in &meshes {
            form::kv(ui, file, |ui| {
                if let Some(triangles) = triangles {
                    ui.label(format!("{triangles} triangles"));
                }
                if let Some(weight) = weight {
                    let ink = if *weight > MAX_PART_BYTES {
                        theme::DANGER
                    } else {
                        theme::MUTED
                    };
                    ui.colored_label(ink, format!("{weight} of {MAX_PART_BYTES}"));
                }
            });
        }
        if meshes.is_empty() {
            ui.label(theme::muted("no mesh yet: no body can wear the part"));
        }
        ui.add_space(4.0);
        form::hint(
            ui,
            "Pick each mesh by its path in the form: one every build wears, and one for each build \
             cut its own way.",
        );
    });
    ui.add_space(8.0);
    crate::shell::row_form::usages::show_usages_grouped(editor, ui, key, true);
}
