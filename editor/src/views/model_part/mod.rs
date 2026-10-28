//! The model part view: each mesh of the part standing on its skeleton's rig, and how it fits the
//! rig joint by joint. Its meshes are glTF files of the game folder, picked by path in its form.

pub mod fit;

use std::collections::HashMap;
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
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct ModelPartViews {
    pub views: HashMap<String, ModelPartView>,
}

pub struct ModelPartView {
    /// The mesh shown: the one cut for this build, or with `None` the one every build wears.
    pub mesh: Option<BuildName>,
    pub stage: stage::StageState,
}

impl ModelPartViews {
    pub fn view(&self, key: &str) -> &ModelPartView {
        self.views
            .get(key)
            .expect("ensure_view ran before the model part view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut ModelPartView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the model part view was changed")
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ModelPartDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key, &row);
    sidebar::row(ui, "model_part", |ui| {
        row_form::show_row(editor, ui, ModelPartDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "model_part", |editor, ui| {
        inspect(editor, ui, key, &row);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            mesh_bar(editor, ui, key, &row);
            ui.add_space(4.0);
            fit::show(editor, ui, key, &row);
        });
}

pub fn ensure_view(editor: &mut Editor, key: &str, row: &ModelPartDef) {
    if editor.views.model_part.views.contains_key(key) {
        return;
    }
    let mesh = match &row.mesh {
        Some(_) => None,
        None => row.builds.keys().next().cloned(),
    };
    editor.views.model_part.views.insert(
        key.to_owned(),
        ModelPartView {
            mesh,
            stage: stage::StageState::framing(1.3, false),
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
pub fn idle_clip(skeleton: &BuiltSkeleton) -> Option<StageClip> {
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
            let stage = &mut editor.views.model_part.view_mut(key).stage;
            if chip::toggle(ui, "turning", stage.spinning).clicked() {
                stage.spinning = !stage.spinning;
            }
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
    let standing = stage::standing(body, idle_clip(skeleton), None, Vec::new(), Vec::new());
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
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Meshes", "", |_| {});
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
