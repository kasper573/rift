//! Importing a rig over the model: the new glTF's clips matched to the actions and poses by
//! name, the cues that still fit kept, the hitbox and busts untouched, all in one step once the
//! designer has settled every slot.

use std::collections::BTreeMap;

use engine::core::assets::FileSize;
use engine::core::content::gltf::{self, GltfInfo};
use engine::core::time::Millis;
use engine::systems::actor::model::{
    Clip, Cue, MAX_CHARACTER_BYTES, MAX_CHARACTER_TRIANGLES, MAX_JOINTS, MIN_CLIP, ModelDef,
};
use engine::systems::actor::{Action, Pose};
use strum::VariantArray;
use ui::egui::{self, RichText};
use ui::{controls, dialog, theme};

use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;

/// A rig offered as the model's new glTF, with the clip each action and pose would play.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDraft {
    pub source: String,
    pub bytes: Vec<u8>,
    pub info: GltfInfo,
    pub actions: BTreeMap<Action, Option<String>>,
    pub poses: BTreeMap<Pose, Option<String>>,
}

impl ImportDraft {
    /// The clips of the rig nothing plays.
    pub fn unmatched(&self) -> Vec<&str> {
        self.info
            .clip_names()
            .into_iter()
            .filter(|name| {
                !self
                    .actions
                    .values()
                    .chain(self.poses.values())
                    .any(|chosen| chosen.as_deref() == Some(name))
            })
            .collect()
    }

    /// Why the rig cannot be imported as it stands, if it cannot.
    pub fn refusals(&self) -> Vec<String> {
        let mut refusals = Vec::new();
        for (action, chosen) in &self.actions {
            match chosen {
                None => refusals.push(format!("{} plays no clip", action.as_ref())),
                Some(name) => {
                    if let Some(clip) = self.info.clip(name)
                        && clip.length.millis() < MIN_CLIP
                    {
                        refusals.push(format!(
                            "{name} plays {} ms, under {} ms",
                            clip.length.millis().0,
                            MIN_CLIP.0
                        ));
                    }
                }
            }
        }
        if self.info.triangles > MAX_CHARACTER_TRIANGLES {
            refusals.push(format!(
                "{} triangles, over {MAX_CHARACTER_TRIANGLES}",
                self.info.triangles
            ));
        }
        if self.info.joints > MAX_JOINTS {
            refusals.push(format!("{} joints, over {MAX_JOINTS}", self.info.joints));
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

/// `bytes` read as a rig for the model at `key`: every action and pose keeps its clip when the rig
/// has one of that name, or takes the rig's clip whose name says what it is.
pub fn draft(
    editor: &Editor,
    key: &str,
    bytes: Vec<u8>,
    source: String,
) -> Result<ImportDraft, String> {
    let info = gltf::inspect(&bytes).map_err(|error| format!("{source}: {error}"))?;
    let Some(Ok(model)) = editor.read::<ModelDef>(key) else {
        return Err(format!("{} does not read", shown_path(key)));
    };
    let names = info.clip_names();
    let keep = |current: Option<&Clip>| {
        current
            .map(|clip| clip.name.as_str())
            .filter(|name| names.contains(name))
            .map(str::to_owned)
    };
    let actions = Action::VARIANTS
        .iter()
        .map(|action| {
            let chosen =
                keep(model.clips.get(action)).or_else(|| matched(&names, action_names(*action)));
            (*action, chosen)
        })
        .collect();
    let poses = Pose::VARIANTS
        .iter()
        .filter(|pose| **pose != Pose::Idle)
        .map(|pose| {
            let chosen = keep(model.poses.get(pose)).or_else(|| {
                model
                    .poses
                    .contains_key(pose)
                    .then(|| matched(&names, pose_names(*pose)))
                    .flatten()
                    .or_else(|| matched(&names, pose_names(*pose)))
            });
            (*pose, chosen)
        })
        .collect();
    Ok(ImportDraft {
        source,
        bytes,
        info,
        actions,
        poses,
    })
}

/// The clip among `names` that `wanted` describes: one named exactly so, else the shortest name
/// holding one of the words.
pub fn matched(names: &[&str], wanted: &[&str]) -> Option<String> {
    let lower = |name: &str| name.to_lowercase();
    if let Some(exact) = names
        .iter()
        .find(|name| wanted.iter().any(|word| lower(name) == *word))
    {
        return Some((*exact).to_owned());
    }
    names
        .iter()
        .filter(|name| wanted.iter().any(|word| lower(name).contains(word)))
        .min_by_key(|name| name.len())
        .map(|name| (*name).to_owned())
}

/// Writes the rig of `draft` as the model's glTF with the clips it chose, as one step.
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
    let length = |name: &str| draft.info.clip(name).map(|clip| clip.length.millis());
    model.clips = draft
        .actions
        .iter()
        .filter_map(|(action, chosen)| {
            let name = chosen.as_ref()?;
            let cues = row
                .clips
                .get(action)
                .map(|clip| fitting(&clip.cues, length(name), action.takes_apex()))
                .unwrap_or_default();
            Some((
                *action,
                Clip {
                    name: name.clone(),
                    cues,
                },
            ))
        })
        .collect();
    model.poses = draft
        .poses
        .iter()
        .filter_map(|(pose, chosen)| {
            let name = chosen.as_ref()?;
            let cues = row
                .poses
                .get(pose)
                .map(|clip| fitting(&clip.cues, length(name), false))
                .unwrap_or_default();
            Some((
                *pose,
                Clip {
                    name: name.clone(),
                    cues,
                },
            ))
        })
        .collect();
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    let mut edit = ContentEdit::new(format!("Import a rig into {}", shown_path(key)));
    if let Err(reason) = edit.write_content(project, key, &model) {
        editor.toast(reason);
        return false;
    }
    edit.write(row.model.path(), draft.bytes.clone().into());
    editor.apply(edit)
}

/// The cues of `cues` that still lie within a clip `length` long; a clip that takes an apex and
/// lost it lands halfway through.
pub fn fitting(cues: &[Cue], length: Option<Millis>, takes_apex: bool) -> Vec<Cue> {
    let mut kept: Vec<Cue> = cues
        .iter()
        .filter(|cue| length.is_none_or(|length| cue.at <= length))
        .cloned()
        .collect();
    if takes_apex
        && !kept.iter().any(|cue| cue.apex)
        && let Some(length) = length
    {
        kept.push(Cue {
            at: Millis((length.0 / 2.0 / 10.0).round() * 10.0),
            step: false,
            apex: true,
            sound: None,
        });
        kept.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    }
    kept
}

pub fn dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(mut draft) = editor.views.model.view(key).import.clone() else {
        return;
    };
    let names: Vec<String> = draft
        .info
        .clip_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut decision: Option<bool> = None;
    dialog::card(ctx, "model-import", 520.0, |ui| {
        ui.label(
            RichText::new(format!("Import a rig into {}", shown_path(key)))
                .size(16.0)
                .strong(),
        );
        ui.label(theme::muted(&draft.source));
        ui.add_space(6.0);
        ui.label(theme::muted(format!(
            "{} triangles · {} joints · {} clips · {}",
            draft.info.triangles,
            draft.info.joints,
            names.len(),
            FileSize(draft.bytes.len())
        )));
        ui.add_space(8.0);
        egui::Grid::new("import-slots")
            .num_columns(3)
            .spacing(egui::Vec2::new(10.0, 4.0))
            .show(ui, |ui| {
                for action in Action::VARIANTS {
                    let chosen = draft.actions.entry(*action).or_default();
                    slot_row(ui, action.as_ref(), chosen, &names, &draft.info);
                    ui.end_row();
                }
                for pose in Pose::VARIANTS.iter().filter(|pose| **pose != Pose::Idle) {
                    let chosen = draft.poses.entry(*pose).or_default();
                    slot_row(
                        ui,
                        &format!("{} pose", pose.as_ref()),
                        chosen,
                        &names,
                        &draft.info,
                    );
                    ui.end_row();
                }
            });
        let unmatched = draft.unmatched();
        if !unmatched.is_empty() {
            ui.add_space(6.0);
            ui.label(theme::muted(format!(
                "Nothing plays {}.",
                unmatched.join(", ")
            )));
        }
        let refusals = draft.refusals();
        for refusal in &refusals {
            ui.colored_label(theme::DANGER, refusal);
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
        None => editor.views.model.view_mut(key).import = Some(draft),
    }
}

fn slot_row(
    ui: &mut egui::Ui,
    label: &str,
    chosen: &mut Option<String>,
    names: &[String],
    info: &GltfInfo,
) {
    ui.label(label);
    controls::combo(("import-slot", label))
        .selected_text(chosen.clone().unwrap_or_else(|| "none".to_owned()))
        .width(200.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(chosen, None, "none");
            for name in names {
                ui.selectable_value(chosen, Some(name.clone()), name);
            }
        });
    match chosen.as_deref().and_then(|name| info.clip(name)) {
        Some(clip) => {
            ui.label(theme::muted(format!("{:.2} s", clip.length.0)));
        }
        None => {
            ui.label(theme::muted("–"));
        }
    }
}

fn action_names(action: Action) -> &'static [&'static str] {
    match action {
        Action::Idle => &["idle", "stand", "breath", "rest"],
        Action::Walk => &["walk", "stroll"],
        Action::Run => &["run", "jog", "sprint"],
        Action::Attack => &[
            "attack", "strike", "swing", "slash", "punch", "hit", "shoot", "cast",
        ],
        Action::Dead => &["death", "die", "dead", "dying"],
        Action::Talk => &["talk", "speak", "interact", "gesture", "wave"],
    }
}

fn pose_names(pose: Pose) -> &'static [&'static str] {
    match pose {
        Pose::Idle => &["idle"],
        Pose::Sit => &["sit", "seated", "sitting"],
    }
}
