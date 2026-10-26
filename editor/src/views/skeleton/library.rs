//! A glb dropped on a skeleton: offered as a library of clips made on its rig, or as the rig in
//! place of the one it has. A library must fit the rig as a model must; its clips are new, already
//! in a library (the designer picks which keeps the name) or utility clips no set will name, which
//! are left out, as are its meshes and skins. A new rig must fit every model and library already on
//! the skeleton.

use std::collections::{BTreeMap, BTreeSet};

use engine::core::assets::FileSize;
use engine::core::content::files::valid_name;
use engine::core::content::gltf::{self, GltfInfo};
use engine::systems::actor::skeleton::{
    BuiltSkeleton, CLIP_LIBRARIES, MAX_CLIP, MAX_LIBRARIES_BYTES, MAX_LIBRARY_BYTES, MIN_CLIP,
    RIG_GLB, misfit_problem,
};
use ui::egui::{self, RichText};
use ui::{dialog, theme};

use crate::app::Editor;
use crate::project::ContentEdit;
use crate::rows::name_from;
use crate::tabs::shown_path;

/// A glb dropped on a skeleton, read and fitted both ways: as a library and as the rig.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryDraft {
    pub source: String,
    pub bytes: Vec<u8>,
    pub info: GltfInfo,
    /// The library's name: its file is `clips/<name>.glb`.
    pub name: String,
    pub as_rig: bool,
    /// How its joints fit the rig, and the loader's word on each that does not.
    pub matched: usize,
    pub joints: usize,
    pub worst_rest: Option<(String, f32)>,
    pub misfits: Vec<String>,
    pub clips: Vec<DraftClip>,
    /// The clips already in another library that this one takes over.
    pub taken: BTreeSet<String>,
    /// How the skeleton's models and libraries fit the glb as a rig.
    pub refit: Vec<String>,
    /// What the library weighs once cut to the clips it adds, or why it cannot be cut.
    pub cut: Result<FileSize, String>,
    /// What the skeleton's other libraries weigh, those it takes clips from as they would be left.
    pub others: FileSize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DraftClip {
    pub name: String,
    pub length_ms: f32,
    pub kind: ClipKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipKind {
    New,
    /// Already carried by the library at this key.
    Present(String),
    /// Left out: why no set will name it.
    Utility(String),
}

impl LibraryDraft {
    /// The clips the library keeps: those new, and those it takes over.
    pub fn kept(&self) -> Vec<&str> {
        self.clips
            .iter()
            .filter(|clip| match &clip.kind {
                ClipKind::New => true,
                ClipKind::Present(_) => self.taken.contains(&clip.name),
                ClipKind::Utility(_) => false,
            })
            .map(|clip| clip.name.as_str())
            .collect()
    }

    /// Why the glb cannot be added as it stands, if it cannot.
    pub fn refusals(&self) -> Vec<String> {
        if self.as_rig {
            return self.refit.clone();
        }
        let mut refusals = self.misfits.clone();
        if !valid_name(&self.name) {
            refusals.push(format!(
                "clips/{}.glb: a name is {}",
                self.name,
                engine::core::content::files::NAME_RULE
            ));
        }
        if self.kept().is_empty() {
            refusals
                .push("no clip to add: every one is a utility clip or kept elsewhere".to_owned());
        }
        match &self.cut {
            Ok(weight) => {
                if *weight > MAX_LIBRARY_BYTES {
                    refusals.push(format!(
                        "the library weighs {weight}, over {MAX_LIBRARY_BYTES}"
                    ));
                }
                let total = FileSize(weight.0 + self.others.0);
                if total > MAX_LIBRARIES_BYTES {
                    refusals.push(format!(
                        "libraries weigh {total}, over {MAX_LIBRARIES_BYTES}"
                    ));
                }
            }
            Err(error) => refusals.push(error.clone()),
        }
        refusals
    }
}

/// `bytes` read as a library for the skeleton at `key`, named after `name`, fitted against its rig
/// and as its rig.
pub fn draft(
    editor: &Editor,
    key: &str,
    bytes: Vec<u8>,
    source: String,
    name: &str,
) -> Result<LibraryDraft, String> {
    let info = gltf::inspect(&bytes).map_err(|error| format!("{source}: {error}"))?;
    let (content, _, built) = super::built(editor, key).ok_or_else(|| {
        format!(
            "{} did not load: fix it before adding to it",
            shown_path(key)
        )
    })?;
    let fit = gltf::fit(&built.rig, &info.tree);
    let misfits = fit
        .misfits
        .iter()
        .map(|misfit| {
            let problem = misfit_problem("the dropped glb", "rig.glb", misfit);
            format!("{}; {}", problem.found, problem.expected)
        })
        .collect();
    let mut seen = BTreeSet::new();
    let clips = info
        .clips
        .iter()
        .map(|clip| DraftClip {
            name: clip.name.clone(),
            length_ms: clip.length.millis().0,
            kind: kind(&built, &clip.name, clip.length.millis().0, &mut seen),
        })
        .collect();
    let mut refit = Vec::new();
    for (path, model) in super::wearers(&content, built_id(&content, key)?) {
        let Some(model) = model else {
            continue;
        };
        let Some(skin) = content
            .built::<engine::core::content::media::GltfInfos>()
            .0
            .get(&model.model().path())
        else {
            continue;
        };
        for misfit in &gltf::fit(&info.tree, &skin.tree).misfits {
            let problem = misfit_problem(shown_path(&path), "the new rig", misfit);
            refit.push(format!("{}; {}", problem.found, problem.expected));
        }
    }
    for library in &built.def.libraries {
        let Some(carried) = content
            .built::<engine::core::content::media::GltfInfos>()
            .0
            .get(&library.file.path())
        else {
            continue;
        };
        for misfit in &gltf::fit(&info.tree, &carried.tree).misfits {
            let file = format!("clips/{}.glb", library.key);
            let problem = misfit_problem(&file, "the new rig", misfit);
            refit.push(format!("{}; {}", problem.found, problem.expected));
        }
    }
    let mut draft = LibraryDraft {
        source,
        bytes,
        info,
        name: name_from(name),
        as_rig: false,
        matched: fit.matched,
        joints: fit.joints,
        worst_rest: fit.worst_rest.map(|(joint, turn)| (joint, turn.0)),
        misfits,
        clips,
        taken: BTreeSet::new(),
        refit,
        cut: Ok(FileSize(0)),
        others: FileSize(0),
    };
    weigh(editor, &built, &mut draft);
    Ok(draft)
}

/// Writes the draft: the cut library into `clips/`, and the libraries it takes clips from without
/// them; or the cut rig in place of `rig.glb`. One History step.
pub fn apply(editor: &mut Editor, key: &str, draft: &LibraryDraft) -> bool {
    if let Some(refusal) = draft.refusals().first() {
        editor.toast(format!("{}: {refusal}", shown_path(key)));
        return false;
    }
    let Some((_, _, built)) = super::built(editor, key) else {
        return false;
    };
    let edit = if draft.as_rig {
        let mut edit = ContentEdit::new(format!("Replace the rig of {}", shown_path(key)));
        match gltf::cut(&draft.bytes, &[]) {
            Ok(rig) => {
                edit.write(format!("{key}/{}", RIG_GLB.named("", "glb")), rig.into());
            }
            Err(error) => {
                editor.toast(format!("{}: {error}", draft.source));
                return false;
            }
        }
        edit
    } else {
        let mut edit = ContentEdit::new(format!(
            "Add clips/{}.glb to {}",
            draft.name,
            shown_path(key)
        ));
        match gltf::cut(&draft.bytes, &draft.kept()) {
            Ok(library) => {
                edit.write(library_path(key, &draft.name), library.into());
            }
            Err(error) => {
                editor.toast(format!("{}: {error}", draft.source));
                return false;
            }
        }
        for (library, left) in left_behind(editor, &built, draft) {
            match left {
                Ok(bytes) => {
                    edit.write(library_path(key, &library), bytes.into());
                }
                Err(error) => {
                    editor.toast(format!("clips/{library}.glb: {error}"));
                    return false;
                }
            }
        }
        edit
    };
    editor.apply(edit)
}

pub fn dialog(editor: &mut Editor, ctx: &egui::Context, key: &str) {
    let Some(mut draft) = editor.views.skeleton.view(key).library.clone() else {
        return;
    };
    let before = draft.clone();
    let mut decision: Option<bool> = None;
    dialog::card(ctx, "skeleton-library", 600.0, |ui| {
        let title = if draft.as_rig {
            format!("Replace the rig of {}", shown_path(key))
        } else {
            format!("Add a library to {}", shown_path(key))
        };
        ui.label(RichText::new(title).size(16.0).strong());
        ui.label(theme::muted(format!(
            "{} · {} joints · {} clips · {}",
            draft.source,
            draft.info.joints(),
            draft.info.clips.len(),
            FileSize(draft.bytes.len())
        )));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut draft.as_rig, false, "as a library");
            ui.selectable_value(&mut draft.as_rig, true, "as the rig");
        });
        ui.add_space(6.0);
        if draft.as_rig {
            rig_body(ui, &draft);
        } else {
            library_body(ui, &mut draft);
        }
        let refusals = draft.refusals();
        if !refusals.is_empty() {
            ui.add_space(6.0);
        }
        for refusal in &refusals {
            ui.colored_label(theme::DANGER, refusal);
        }
        if !refusals.is_empty() {
            ui.label(theme::faint(
                "Fix the glb in the pipeline, which renames and re-rests joints and names clips, \
                 then drop it again.",
            ));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let confirm = if draft.as_rig {
                "Replace the rig"
            } else {
                "Add library"
            };
            if ui
                .add_enabled(refusals.is_empty(), egui::Button::new(confirm))
                .clicked()
            {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    if draft != before
        && let Some((_, _, built)) = super::built(editor, key)
    {
        weigh(editor, &built, &mut draft);
    }
    match decision {
        Some(true) => {
            editor.views.skeleton.view_mut(key).library = None;
            apply(editor, key, &draft);
        }
        Some(false) => editor.views.skeleton.view_mut(key).library = None,
        None => editor.views.skeleton.view_mut(key).library = Some(draft),
    }
}

fn library_body(ui: &mut egui::Ui, draft: &mut LibraryDraft) {
    ui.label(RichText::new("fit against rig.glb").strong());
    let mut fits = format!("{} of {} joints", draft.matched, draft.joints);
    if let Some((joint, turn)) = &draft.worst_rest {
        fits.push_str(&format!(", rest worst {joint} {turn:.1}°"));
    }
    ui.label(fits);
    ui.add_space(6.0);
    ui.label(RichText::new("clips").strong());
    let mut taken = draft.taken.clone();
    egui::ScrollArea::vertical()
        .id_salt("library-clips")
        .max_height(260.0)
        .show(ui, |ui| {
            for clip in &draft.clips {
                ui.horizontal(|ui| {
                    ui.label(theme::mono(&clip.name));
                    ui.label(theme::faint(format!("{:.2} s", clip.length_ms / 1000.0)));
                    match &clip.kind {
                        ClipKind::New => {
                            ui.label(theme::muted("new"));
                        }
                        ClipKind::Present(library) => {
                            ui.label(theme::muted(format!("also in clips/{library}.glb")));
                            let mut theirs = taken.contains(&clip.name);
                            ui.selectable_value(&mut theirs, false, format!("keep {library}'s"));
                            ui.selectable_value(&mut theirs, true, "use this one");
                            if theirs {
                                taken.insert(clip.name.clone());
                            } else {
                                taken.remove(&clip.name);
                            }
                        }
                        ClipKind::Utility(why) => {
                            ui.label(theme::faint(format!("{why} · skipped")));
                        }
                    }
                });
            }
        });
    draft.taken = taken;
    ui.add_space(6.0);
    ui.label(RichText::new("after import").strong());
    ui.horizontal(|ui| {
        ui.label(theme::muted("saved as clips/"));
        ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(140.0));
        ui.label(theme::muted(".glb"));
    });
    let weight = match &draft.cut {
        Ok(weight) => format!(
            "{} clips · {weight} · libraries {} of {MAX_LIBRARIES_BYTES}",
            draft.kept().len(),
            FileSize(weight.0 + draft.others.0)
        ),
        Err(_) => "cannot be cut".to_owned(),
    };
    ui.label(weight);
    ui.label(theme::faint(
        "No set cell changes until a set names the new clips (the sets page).",
    ));
}

fn rig_body(ui: &mut egui::Ui, draft: &LibraryDraft) {
    ui.label(RichText::new("every model and library refitted").strong());
    if draft.refit.is_empty() {
        ui.label("every model wearing the skeleton and every library fits it");
    }
    ui.label(theme::faint(
        "The rig keeps only its joints at rest: its meshes, skins and clips are left out.",
    ));
}

/// What a clip of the dropped glb is to the skeleton: new, already in one of its libraries, or a
/// clip no set will name.
fn kind(
    built: &BuiltSkeleton,
    name: &str,
    length_ms: f32,
    seen: &mut BTreeSet<String>,
) -> ClipKind {
    let squeezed: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    if name.is_empty() {
        return ClipKind::Utility("unnamed".to_owned());
    }
    if [
        "tpose",
        "apose",
        "bind",
        "bindpose",
        "rest",
        "restpose",
        "reference",
    ]
    .contains(&squeezed.as_str())
    {
        return ClipKind::Utility("a pose to rig against".to_owned());
    }
    if length_ms < MIN_CLIP.0 || length_ms > MAX_CLIP.0 {
        return ClipKind::Utility(format!(
            "{length_ms:.0} ms, not {} ms to {} ms",
            MIN_CLIP.0, MAX_CLIP.0
        ));
    }
    if !seen.insert(name.to_owned()) {
        return ClipKind::Utility("a second clip of the name".to_owned());
    }
    let clip = engine::systems::actor::skeleton::ClipName::new(name);
    match built.clips.get(&clip) {
        Some(found) => ClipKind::Present(built.def.libraries[found.library].key.clone()),
        None => ClipKind::New,
    }
}

/// Weighs the library as it would be cut, and the skeleton's other libraries as they would be left.
fn weigh(editor: &Editor, built: &BuiltSkeleton, draft: &mut LibraryDraft) {
    draft.cut = if draft.as_rig {
        Ok(FileSize(0))
    } else {
        gltf::cut(&draft.bytes, &draft.kept())
            .map(|bytes| FileSize(bytes.len()))
            .map_err(|error| format!("{}: {error}", draft.source))
    };
    let left: BTreeMap<String, usize> = left_behind(editor, built, draft)
        .into_iter()
        .filter_map(|(library, bytes)| Some((library, bytes.ok()?.len())))
        .collect();
    draft.others = FileSize(
        built
            .def
            .libraries
            .iter()
            .filter(|library| library.key != draft.name)
            .map(|library| {
                left.get(&library.key).copied().unwrap_or_else(|| {
                    super::weight(editor, &library.file.path()).map_or(0, |weight| weight.0)
                })
            })
            .sum(),
    );
}

/// Each library the draft takes clips from, cut to the clips it keeps.
fn left_behind(
    editor: &Editor,
    built: &BuiltSkeleton,
    draft: &LibraryDraft,
) -> Vec<(String, Result<Vec<u8>, String>)> {
    let mut losing: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (name, found) in &built.clips {
        if draft.taken.contains(name.as_str()) {
            losing.entry(found.library).or_default().push(name.as_str());
        }
    }
    losing
        .into_iter()
        .map(|(library, lost)| {
            let def = &built.def.libraries[library];
            let kept: Vec<&str> = built
                .clips
                .iter()
                .filter(|(name, found)| found.library == library && !lost.contains(&name.as_str()))
                .map(|(name, _)| name.as_str())
                .collect();
            let bytes = editor
                .project
                .as_ref()
                .and_then(|project| project.files.get(&def.file.path()))
                .ok_or_else(|| "the library's file is gone".to_owned())
                .and_then(|bytes| gltf::cut(bytes, &kept));
            (def.key.clone(), bytes)
        })
        .collect()
}

fn library_path(key: &str, name: &str) -> String {
    format!("{key}/{}", CLIP_LIBRARIES.named(name, "glb"))
}

fn built_id(
    content: &engine::core::content::Content,
    key: &str,
) -> Result<engine::systems::actor::skeleton::SkeletonId, String> {
    content
        .by_path::<engine::systems::actor::skeleton::SkeletonDef>(key)
        .ok_or_else(|| format!("{} is not a skeleton", shown_path(key)))
}
