//! Importing a glb over the model: a fit report against the skeleton the model wears, the clips
//! it carries and what each would do, refused while any joint does not fit. Overrides whose clip
//! the new glb still carries keep the cues that still lie within it; the hitbox and busts stay.

use std::collections::BTreeMap;

use engine::core::assets::FileSize;
use engine::core::content::gltf::{self, GltfInfo};
use engine::systems::actor::model::{
    MAX_CHARACTER_BYTES, MAX_CHARACTER_TRIANGLES, ModelDef, fit_report,
};
use engine::systems::actor::skeleton::{ClipName, Cue};
use ui::egui::{self, RichText};
use ui::{dialog, theme};

use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;
use crate::views::cues;

/// A glb offered as the model's new skin, with how it fits the model's skeleton.
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
    /// Every clip the glb carries, with what it would do.
    pub carried: Vec<(String, String)>,
}

impl ImportDraft {
    /// Why the glb cannot be imported as it stands, if it cannot.
    pub fn refusals(&self) -> Vec<String> {
        let mut refusals = self.misfits.clone();
        if self.info.triangles > MAX_CHARACTER_TRIANGLES {
            refusals.push(format!(
                "{} triangles, over {MAX_CHARACTER_TRIANGLES}",
                self.info.triangles
            ));
        }
        if FileSize(self.bytes.len()) > MAX_CHARACTER_BYTES {
            refusals.push(format!(
                "{} of model, over {MAX_CHARACTER_BYTES}",
                FileSize(self.bytes.len())
            ));
        }
        refusals
    }
}

/// `bytes` read as a skin for the model at `key`, fitted against the skeleton it wears.
pub fn draft(
    editor: &Editor,
    key: &str,
    bytes: Vec<u8>,
    source: String,
) -> Result<ImportDraft, String> {
    let info = gltf::inspect(&bytes).map_err(|error| format!("{source}: {error}"))?;
    let (content, model) = super::built(editor, key).ok_or_else(|| {
        format!(
            "{} did not load: fix it before importing over it",
            shown_path(key)
        )
    })?;
    let skeleton = model.skeleton();
    let skeleton_path = model.skeleton_id().path(content);
    let report = fit_report(skeleton, skeleton_path, &info.tree);
    let carried = info
        .clips
        .iter()
        .map(|clip| {
            let name = ClipName::new(clip.name.clone());
            let named = skeleton
                .def
                .sets()
                .any(|(_, set)| set.entries().any(|(_, named)| *named.clip() == name));
            let note = match skeleton.clips.get(&name) {
                _ if !named => "no set names it · never plays".to_owned(),
                Some(library) => format!(
                    "would override clips/{}.glb's {name}",
                    skeleton.def.libraries[library.library].key
                ),
                None => "plays where a set names it".to_owned(),
            };
            (clip.name.clone(), note)
        })
        .collect();
    Ok(ImportDraft {
        source,
        info,
        bytes,
        skeleton: engine::core::content::name_of(skeleton_path).to_owned(),
        matched: report.fit.matched,
        joints: report.fit.joints,
        worst_rest: report.fit.worst_rest.map(|(joint, turn)| (joint, turn.0)),
        misfits: report
            .problems
            .iter()
            .map(|problem| format!("{}; {}", problem.found, problem.expected))
            .collect(),
        carried,
    })
}

/// Writes the glb of `draft` as the model's, keeping the overrides it still carries, as one step.
pub fn apply(editor: &mut Editor, key: &str, draft: &ImportDraft) -> bool {
    let refusals = draft.refusals();
    if let Some(first) = refusals.first() {
        editor.toast(format!("{}: {first}", shown_path(key)));
        return false;
    }
    let Some(Ok(row)) = editor.read::<ModelDef>(key) else {
        return false;
    };
    let mut model = (*row).clone();
    model.overrides = row
        .overrides
        .iter()
        .filter_map(|(clip, held)| {
            let length = draft.info.clip(&clip.0)?.length.millis();
            Some((clip.clone(), cues::fitting(held, length)))
        })
        .filter(|(_, held): &(ClipName, Vec<Cue>)| !held.is_empty())
        .collect::<BTreeMap<ClipName, Vec<Cue>>>();
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut edit = ContentEdit::new(format!("Import a skin into {}", shown_path(key)));
    if let Err(reason) = edit.write_content(project, key, &model) {
        editor.toast(reason);
        return false;
    }
    edit.write(row.model.path(), draft.bytes.clone().into());
    editor.apply(edit)
}

pub fn dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(draft) = editor.views.model.view(key).import.clone() else {
        return;
    };
    let mut decision: Option<bool> = None;
    dialog::card(ctx, "model-import", 560.0, |ui| {
        ui.label(
            RichText::new(format!("Import a skin into {}", shown_path(key)))
                .size(16.0)
                .strong(),
        );
        ui.label(theme::muted(format!(
            "{} · {} triangles · {} joints · {} clips · {}",
            draft.source,
            draft.info.triangles,
            draft.info.joints(),
            draft.info.clips.len(),
            FileSize(draft.bytes.len())
        )));
        ui.add_space(8.0);
        ui.label(RichText::new(format!("fit against {}.skeleton", draft.skeleton)).strong());
        ui.label(format!(
            "joints {} of {} matched",
            draft.matched, draft.joints
        ));
        if let Some((joint, turn)) = &draft.worst_rest {
            ui.label(theme::muted(format!("rest worst {joint}, {turn:.1}°")));
        }
        if !draft.carried.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("clips it carries").strong());
            for (clip, note) in &draft.carried {
                ui.label(format!("{clip} · {note}"));
            }
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
            editor.views.model.view_mut(key).import = None;
            apply(editor, key, &draft);
        }
        Some(false) => editor.views.model.view_mut(key).import = None,
        None => {}
    }
}
