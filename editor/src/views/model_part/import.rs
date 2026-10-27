//! Importing a glb as a mesh of the part: a fit report against the part's skeleton, refused while
//! any joint does not fit or it carries clips, written for every build or for one.

use engine::core::assets::FileSize;
use engine::core::content::gltf::{self, GltfInfo};
use engine::systems::actor::model::fit_report;
use engine::systems::actor::part::{BUILD_MESHES, MAX_PART_BYTES, ModelPartDef, PART_MESH};
use engine::systems::actor::skeleton::BuildName;
use ui::egui::{self, RichText};
use ui::{controls, dialog, theme};

use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;

/// A glb offered as a mesh of the part, with how it fits the part's skeleton.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDraft {
    pub source: String,
    pub bytes: Vec<u8>,
    pub info: GltfInfo,
    pub skeleton: String,
    pub matched: usize,
    pub joints: usize,
    /// The worst-resting joint of those that fit, and by how many degrees.
    pub worst_rest: Option<(String, f32)>,
    /// The loader's word on every joint that does not fit, which refuses the import.
    pub misfits: Vec<String>,
    /// The builds of the skeleton the mesh may be cut for.
    pub builds: Vec<BuildName>,
    /// The build the mesh is imported for, or with `None` every build.
    pub build: Option<BuildName>,
}

impl ImportDraft {
    /// Why the glb cannot be imported as it stands, if it cannot.
    pub fn refusals(&self) -> Vec<String> {
        let mut refusals = self.misfits.clone();
        if !self.info.clips.is_empty() {
            refusals.push(format!(
                "carries clips ({}): a body plays only its skeleton's",
                self.info.clip_names().join(", ")
            ));
        }
        if FileSize(self.bytes.len()) > MAX_PART_BYTES {
            refusals.push(format!(
                "{} of mesh, over {MAX_PART_BYTES}",
                FileSize(self.bytes.len())
            ));
        }
        refusals
    }

    /// Where in the part's bundle the mesh goes.
    pub fn part(&self) -> String {
        match &self.build {
            Some(build) => BUILD_MESHES.named(build.as_str(), "glb"),
            None => PART_MESH.named("", "glb"),
        }
    }
}

/// `bytes` read as a mesh of the part at `key`, fitted against its skeleton, for the build its
/// view shows.
pub fn draft(
    editor: &Editor,
    key: &str,
    bytes: Vec<u8>,
    source: String,
) -> Result<ImportDraft, String> {
    let info = gltf::inspect(&bytes).map_err(|error| format!("{source}: {error}"))?;
    let row = editor
        .read::<ModelPartDef>(key)
        .and_then(Result::ok)
        .ok_or_else(|| format!("{} doesn't read: fix it before importing", shown_path(key)))?;
    let (content, skeleton) = super::skeleton(editor, &row).ok_or_else(|| {
        format!(
            "the skeleton of {} did not load: fix it before importing",
            shown_path(key)
        )
    })?;
    let skeleton_path = row.skeleton.path(content);
    let report = fit_report(skeleton, skeleton_path, &source, &info);
    Ok(ImportDraft {
        skeleton: engine::core::content::name_of(skeleton_path).to_owned(),
        matched: report.fit.matched,
        joints: report.fit.joints,
        worst_rest: report.fit.worst_rest.map(|(joint, turn)| (joint, turn.0)),
        misfits: report
            .problems
            .iter()
            .map(|problem| format!("{}; {}", problem.found, problem.expected))
            .collect(),
        builds: skeleton.def.builds.clone(),
        build: editor.views.model_part.view(key).mesh.clone(),
        source,
        info,
        bytes,
    })
}

/// Writes the glb of `draft` as the part's mesh for its build, as one step.
pub fn apply(editor: &mut Editor, key: &str, draft: &ImportDraft) -> bool {
    let refusals = draft.refusals();
    if let Some(first) = refusals.first() {
        editor.toast(format!("{}: {first}", shown_path(key)));
        return false;
    }
    let mut edit = ContentEdit::new(format!("Import {} into {}", draft.part(), shown_path(key)));
    edit.write(
        format!("{key}/{}", draft.part()),
        draft.bytes.clone().into(),
    );
    let applied = editor.apply(edit);
    if applied {
        editor.views.model_part.view_mut(key).mesh = draft.build.clone();
    }
    applied
}

pub fn dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(mut draft) = editor.views.model_part.view(key).import.clone() else {
        return;
    };
    let mut decision: Option<bool> = None;
    dialog::card(ctx, "part-import", 560.0, |ui| {
        ui.label(
            RichText::new(format!("Import a mesh into {}", shown_path(key)))
                .size(16.0)
                .strong(),
        );
        ui.label(theme::muted(format!(
            "{} · {} triangles · {} joints · {}",
            draft.source,
            draft.info.triangles,
            draft.info.joints(),
            FileSize(draft.bytes.len())
        )));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(theme::muted("for"));
            let label = |build: &Option<BuildName>| {
                build
                    .as_ref()
                    .map_or("every build".to_owned(), |build| build.to_string())
            };
            controls::combo("part-import-build")
                .selected_text(label(&draft.build))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.build, None, label(&None));
                    for build in &draft.builds {
                        let some = Some(build.clone());
                        ui.selectable_value(&mut draft.build, some.clone(), label(&some));
                    }
                });
            ui.label(theme::faint(draft.part()));
        });
        ui.add_space(6.0);
        ui.label(RichText::new(format!("fit against {}.skeleton", draft.skeleton)).strong());
        ui.label(format!(
            "joints {} of {} skinned matched",
            draft.matched, draft.joints
        ));
        if let Some((joint, turn)) = &draft.worst_rest {
            ui.label(theme::muted(format!("rest worst {joint}, {turn:.1}°")));
        }
        let refusals = draft.refusals();
        if !refusals.is_empty() {
            ui.add_space(6.0);
        }
        for refusal in &refusals {
            ui.colored_label(theme::DANGER, refusal);
        }
        if !draft.misfits.is_empty() {
            ui.label(theme::faint(
                "Fix the rig in the pipeline, which renames and re-rests its joints, then drop it \
                 again.",
            ));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(refusals.is_empty(), egui::Button::new("Import"))
                .clicked()
            {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    match decision {
        Some(true) => {
            editor.views.model_part.view_mut(key).import = None;
            apply(editor, key, &draft);
        }
        Some(false) => editor.views.model_part.view_mut(key).import = None,
        None => editor.views.model_part.view_mut(key).import = Some(draft),
    }
}
