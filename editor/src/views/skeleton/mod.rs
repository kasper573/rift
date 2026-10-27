//! The skeleton view: the rig with its hands and groups marked, what plays for each action, pose
//! and move in each stance, the clips of its libraries with their cues, the hands items hang from,
//! what a body is put together and painted from, and the layers that move a body beyond its clips.
//! The stage shows the rig as any model wearing it, playing any clip, holding any item, dressed in
//! any worn items. A glb dropped on the view is offered as a new library, or as the rig in place of
//! the one it has; a folder of part meshes, as the skeleton's parts.

pub mod body;
pub mod clips;
pub mod hands;
pub mod import;
pub mod inspect;
pub mod joints;
pub mod layers;
pub mod library;
pub mod sets;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::Transform;
use engine::core::assets::FileSize;
use engine::core::content::{Content, ContentFile, NamedContent};
use engine::core::render::world::mannequin::RigClip;
use engine::core::time::Millis;
use engine::core::units::MetersPerSec;
use engine::systems::actor::model::{ActorModel, Skeletons};
use engine::systems::actor::skeleton::{
    BuiltSkeleton, ClipName, Cue, EquipmentSlot, GroupName, Hand, JointName, Layer, LocalOffset,
    LocalTurn, SkeletonDef, SkeletonId,
};
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Action, Appearance, ModelDef, Motion, Pose, appearance, held};
use engine::systems::item::{Grip, ItemDef, Wear};
use serde::Deserialize;
use ui::egui::{self, Rect, Ui, Vec2};
use ui::{chip, controls, theme, words};

use super::area::camera::AreaCanvas;
use super::area::viewport::{StageLayers, StandingProp};
use super::stage;
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct SkeletonViews {
    pub views: HashMap<String, SkeletonView>,
    pub drops: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkeletonPage {
    #[default]
    Joints,
    Sets,
    Clips,
    Hands,
    Body,
    Layers,
}

impl SkeletonPage {
    const ALL: [SkeletonPage; 6] = [
        SkeletonPage::Joints,
        SkeletonPage::Sets,
        SkeletonPage::Clips,
        SkeletonPage::Hands,
        SkeletonPage::Body,
        SkeletonPage::Layers,
    ];

    fn label(self) -> &'static str {
        match self {
            SkeletonPage::Joints => "joints",
            SkeletonPage::Sets => "sets",
            SkeletonPage::Clips => "clips",
            SkeletonPage::Hands => "hands",
            SkeletonPage::Body => "body",
            SkeletonPage::Layers => "layers",
        }
    }
}

pub struct SkeletonView {
    pub page: SkeletonPage,
    /// The model the stage shows the rig as, by its file's path.
    pub wearer: Option<String>,
    /// The clip the stage plays; none stands the rig at rest.
    pub clip: Option<ClipName>,
    pub scrub: super::cues::Scrub,
    /// The joint lit on the joints page.
    pub joint: Option<String>,
    /// The hand the hands page edits, by its place in the list.
    pub hand: usize,
    /// The item the stage hangs from the model's hands, by its file's path.
    pub trying: Option<String>,
    /// The items the stage's model wears, by the slot each is worn in and its file's path.
    pub wearing: BTreeMap<EquipmentSlot, String>,
    pub sets: sets::SetsState,
    pub layers: layers::LayersState,
    pub unused_only: bool,
    pub library: Option<library::LibraryDraft>,
    pub reduce_motion: bool,
    pub stage: stage::StageState,
}

impl SkeletonViews {
    pub fn view(&self, key: &str) -> &SkeletonView {
        self.views
            .get(key)
            .expect("ensure_view ran before the skeleton view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut SkeletonView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the skeleton view was changed")
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<SkeletonDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    take_drops(editor, key);
    let ctx = ui.ctx().clone();
    library::dialog(editor, &ctx, key);
    let page = editor.views.skeleton.view(key).page;
    sidebar::row(ui, "skeleton", |ui| {
        row_form::show_row(editor, ui, SkeletonDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "skeleton", |editor, ui| {
        inspect::show(editor, ui, key, &row, page);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(editor, ui, key, page);
            ui.add_space(4.0);
            let Some((content, id, built)) = built(editor, key) else {
                ui.label(theme::muted(
                    "The skeleton did not load: the output panel says why.",
                ));
                return;
            };
            match page {
                SkeletonPage::Joints => joints::show(editor, ui, key, &content, &built),
                SkeletonPage::Sets => sets::show(editor, ui, key, &content, &built),
                SkeletonPage::Clips => clips::show(editor, ui, key, &content, id, &built),
                SkeletonPage::Hands => hands::show(editor, ui, key, &content, &built),
                SkeletonPage::Body => body::show(editor, ui, key, &content, &built),
                SkeletonPage::Layers => layers::show(editor, ui, key, &content, &built),
            }
        });
}

fn page_bar(editor: &mut Editor, ui: &mut Ui, key: &str, page: SkeletonPage) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        for other in SkeletonPage::ALL {
            if chip::mode(ui, other.label(), page == other).clicked() {
                let view = editor.views.skeleton.view_mut(key);
                view.page = other;
                view.scrub.restart();
            }
        }
        if !matches!(page, SkeletonPage::Sets | SkeletonPage::Layers) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let stage = &mut editor.views.skeleton.view_mut(key).stage;
                if chip::toggle(ui, "turning", stage.spinning).clicked() {
                    stage.spinning = !stage.spinning;
                }
            });
        }
    });
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.skeleton.views.contains_key(key) {
        return;
    }
    editor.views.skeleton.views.insert(
        key.to_owned(),
        SkeletonView {
            page: SkeletonPage::default(),
            wearer: None,
            clip: None,
            scrub: super::cues::Scrub::default(),
            joint: None,
            hand: 0,
            trying: None,
            wearing: BTreeMap::new(),
            sets: sets::SetsState::default(),
            layers: layers::LayersState::default(),
            unused_only: false,
            library: None,
            reduce_motion: false,
            stage: stage::StageState::framing(1.3, false),
        },
    );
}

/// The skeleton at `key` as the game built it from the files as they last loaded, when it loaded.
pub fn built(editor: &Editor, key: &str) -> Option<(Content, SkeletonId, Arc<BuiltSkeleton>)> {
    let content = editor.loaded()?;
    let id = content.by_path::<SkeletonDef>(key)?;
    let built = Skeletons::loaded(&content, id)?.clone();
    Some((content, id, built))
}

/// The models that wear the skeleton `id`, by their paths, with how each was built if it loaded.
pub fn wearers(content: &Content, id: SkeletonId) -> Vec<(String, Option<&ActorModel>)> {
    content
        .table::<ModelDef>()
        .iter()
        .filter(|(_, model)| model.skeleton == id)
        .map(|(model, _)| {
            (
                model.path(content).to_owned(),
                ActorModel::loaded(content, model),
            )
        })
        .collect()
}

/// The items held in a hand, by their paths, with their names.
pub fn hand_items(content: &Content) -> Vec<(String, String)> {
    content
        .table::<ItemDef>()
        .iter()
        .filter(|(_, item)| matches!(item.wear(), Some(Wear::Held { .. })))
        .map(|(id, item)| (id.path(content).to_owned(), item.shown_name().to_owned()))
        .collect()
}

/// How much the file at `path` weighs as the editor holds it.
pub fn weight(editor: &Editor, path: &str) -> Option<FileSize> {
    let project = editor.project.as_ref()?;
    Some(FileSize(project.files.get(path)?.len()))
}

/// The library clip `clip` as a stage plays it.
pub fn rig_clip(built: &BuiltSkeleton, clip: &ClipName) -> Option<RigClip> {
    let found = built.clips.get(clip)?;
    Some(RigClip {
        file: built.def.libraries[found.library].file.path(),
        index: found.index,
    })
}

/// The model the stage of the view at `key` shows the rig as: the one picked, else the first that
/// loaded.
fn wearer<'c>(
    editor: &Editor,
    key: &str,
    content: &'c Content,
    id: SkeletonId,
) -> Option<(String, &'c ActorModel)> {
    let picked = editor.views.skeleton.view(key).wearer.clone();
    let wearers = wearers(content, id);
    picked
        .and_then(|picked| {
            wearers
                .iter()
                .find(|(path, model)| *path == picked && model.is_some())
        })
        .or_else(|| wearers.iter().find(|(_, model)| model.is_some()))
        .and_then(|(path, model)| Some((path.clone(), (*model)?)))
}

/// Which model wears the rig on the stage.
fn wearer_picker(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, id: SkeletonId) {
    let wearers = wearers(content, id);
    let mut picked = wearer(editor, key, content, id).map(|(path, _)| path);
    ui.horizontal(|ui| {
        ui.label(theme::muted("worn by"));
        controls::combo(("skeleton-wearer", key))
            .selected_text(picked.as_deref().map_or("nothing fits".to_owned(), |path| {
                shown_path(path).to_owned()
            }))
            .width(180.0)
            .show_ui(ui, |ui| {
                for (path, model) in &wearers {
                    let label = if model.is_some() {
                        shown_path(path).to_owned()
                    } else {
                        format!("{} · does not fit", shown_path(path))
                    };
                    ui.add_enabled_ui(model.is_some(), |ui| {
                        ui.selectable_value(&mut picked, Some(path.clone()), label);
                    });
                }
            });
    });
    let view = editor.views.skeleton.view_mut(key);
    if picked != view.wearer {
        view.wearer = picked;
    }
}

/// The pickers over a page's stage: which model wears the rig, and which clip plays (or the rest
/// pose, when `rest`).
fn stage_pickers(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    built: &BuiltSkeleton,
    rest: bool,
) {
    let Some(id) = content.by_path::<SkeletonDef>(key) else {
        return;
    };
    wearer_picker(editor, ui, key, content, id);
    let mut clip = editor.views.skeleton.view(key).clip.clone();
    ui.horizontal(|ui| {
        ui.label(theme::muted("clip"));
        controls::combo(("skeleton-clip", key))
            .selected_text(clip.as_ref().map_or("rest".to_owned(), |clip| {
                format!(
                    "{clip} · {:.2} s",
                    built.clips.get(clip).map_or(0.0, |found| found.length.0)
                )
            }))
            .width(220.0)
            .show_ui(ui, |ui| {
                if rest {
                    ui.selectable_value(&mut clip, None, "rest");
                }
                for (name, found) in &built.clips {
                    ui.selectable_value(
                        &mut clip,
                        Some(name.clone()),
                        format!("{name} · {:.2} s", found.length.0),
                    );
                }
            });
    });
    let view = editor.views.skeleton.view_mut(key);
    if clip != view.clip {
        view.clip = clip;
        view.scrub.restart();
    }
}

/// The clip a page that always plays stands the rig in: the one picked, else what the skeleton
/// plays at idle, else its first.
fn played_clip(editor: &mut Editor, key: &str, built: &BuiltSkeleton) -> Option<ClipName> {
    let idle = built
        .def
        .named(None, &Motion::Action(Action::Idle), Pose::Idle)
        .0
        .variant(0)
        .clip();
    let view = editor.views.skeleton.view_mut(key);
    let clip = view
        .clip
        .clone()
        .filter(|clip| built.clips.contains_key(clip))
        .or_else(|| built.clips.contains_key(idle).then(|| idle.clone()))
        .or_else(|| built.clips.keys().next().cloned());
    if view.clip != clip {
        view.clip = clip.clone();
        view.scrub.restart();
    }
    clip
}

/// The item the stage hangs from the model's hands.
fn item_picker(editor: &mut Editor, ui: &mut Ui, key: &str, content: &Content, label: &str) {
    let items = hand_items(content);
    let mut trying = editor.views.skeleton.view(key).trying.clone();
    ui.horizontal(|ui| {
        ui.label(theme::muted(label));
        controls::combo(("skeleton-trying", key))
            .selected_text(
                trying
                    .as_deref()
                    .map_or("nothing".to_owned(), |path| shown_path(path).to_owned()),
            )
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut trying, None, "nothing");
                for (path, name) in &items {
                    ui.selectable_value(&mut trying, Some(path.clone()), name);
                }
            });
    });
    editor.views.skeleton.view_mut(key).trying = trying;
}

/// The view's stage over `size`: the rig worn by the picked model, dressed in the items picked on
/// the body page, in the picked clip at the playhead, holding `props`; gives back where it was drawn, to draw over.
#[allow(clippy::too_many_arguments)]
fn stage_rig(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    built: &BuiltSkeleton,
    size: Vec2,
    props: Vec<StandingProp>,
) -> Option<(Rect, AreaCanvas)> {
    let id = content.by_path::<SkeletonDef>(key)?;
    let (path, model) = wearer(editor, key, content, id)?;
    let body = match content.by_path::<ModelDef>(&path) {
        Some(dressed) => model.dressed(
            content,
            &appearance::look(
                content,
                &Appearance::of(dressed),
                &body::wearing(editor, key, content),
            ),
        ),
        None => model.body(),
    };
    let chosen = editor.views.skeleton.view(key).clip.clone();
    let clip = chosen
        .as_ref()
        .and_then(|clip| Some((rig_clip(built, clip)?, built.clips[clip].length.millis())));
    super::cues::advance(
        ui,
        &mut editor.views.skeleton.view_mut(key).scrub,
        clip.as_ref().map(|(_, length)| *length),
    );
    let scrub = &editor.views.skeleton.view(key).scrub;
    let playhead = scrub.playhead;
    let crossed = match (&chosen, &clip) {
        (Some(chosen), Some((_, length))) => scrub.crossed(
            built.def.cues.get(chosen).map_or(&[], Vec::as_slice),
            *length,
        ),
        _ => Vec::new(),
    };
    let turntable = turntable(editor, ui, key, size)?;
    let drawn = (turntable.rect, turntable.canvas.clone());
    let held = clip.as_ref().map(|_| playhead.seconds());
    let standing = stage::standing(body, clip.map(|(clip, _)| clip), held, crossed, props);
    turntable.stand(editor, ui, vec![standing]);
    Some(drawn)
}

/// The view's stage over `size` with `model` placed by `placed`, playing `clip` at `playhead` and
/// sounding the cues it `crossed`, with its skeleton's layers running as `layered` tells them.
#[allow(clippy::too_many_arguments)]
fn stage_walking(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    size: Vec2,
    model: &ActorModel,
    clip: RigClip,
    playhead: Millis,
    crossed: Vec<Cue>,
    placed: Transform,
    layered: StageLayers,
) {
    let mut state = std::mem::take(&mut editor.views.skeleton.view_mut(key).layers.stage);
    let turntable = stage::turntable(
        editor,
        ui,
        &format!("skeleton-walk:{key}"),
        size,
        &mut state,
    );
    editor.views.skeleton.view_mut(key).layers.stage = state;
    let Some(turntable) = turntable else {
        return;
    };
    let mut standing = stage::standing(
        model.body(),
        Some(clip),
        Some(playhead.seconds()),
        crossed,
        Vec::new(),
    );
    standing.transform = placed;
    standing.layers = Some(layered);
    turntable.stand(editor, ui, vec![standing]);
    ui.ctx().request_repaint();
}

fn turntable(editor: &mut Editor, ui: &mut Ui, key: &str, size: Vec2) -> Option<stage::Turntable> {
    let mut state = std::mem::take(&mut editor.views.skeleton.view_mut(key).stage);
    let turntable = stage::turntable(editor, ui, &format!("skeleton:{key}"), size, &mut state);
    editor.views.skeleton.view_mut(key).stage = state;
    turntable
}

/// The props of the item on trial, held by the stage's model as the game would hang it.
fn holding(editor: &Editor, key: &str, content: &Content, id: SkeletonId) -> Vec<StandingProp> {
    let Some((_, model)) = wearer(editor, key, content, id) else {
        return Vec::new();
    };
    let Some(item) = trying(editor, key, content) else {
        return Vec::new();
    };
    let hands = held::listed(content, model.hands().len(), &[item]);
    stage::props(content, model, &hands)
}

/// The prop of the item on trial hung from hand `hand` of the stage's model by its own grip.
fn tried_in_hand(editor: &Editor, key: &str, content: &Content, hand: usize) -> Vec<StandingProp> {
    let Some(id) = content.by_path::<SkeletonDef>(key) else {
        return Vec::new();
    };
    let Some((_, model)) = wearer(editor, key, content, id) else {
        return Vec::new();
    };
    let Some(item) = trying(editor, key, content) else {
        return Vec::new();
    };
    let grip = match item.get(content).wear() {
        Some(Wear::Held { grip, .. }) => *grip,
        _ => Grip::default(),
    };
    stage::prop_in_hand(content, model, HandIndex(hand as u8), item, &grip)
        .into_iter()
        .collect()
}

fn trying(editor: &Editor, key: &str, content: &Content) -> Option<engine::systems::item::ItemId> {
    let trying = editor.views.skeleton.view(key).trying.as_deref()?;
    content.by_path::<ItemDef>(trying)
}

/// Takes the files dropped on the view: a folder's meshes become the skeleton's parts, a glTF
/// binary is offered as a library or the rig.
fn take_drops(editor: &mut Editor, key: &str) {
    let drops = std::mem::take(&mut editor.views.skeleton.drops);
    if let Some(folder) = drops.iter().find(|path| path.is_dir()) {
        match import::import(editor, key, folder) {
            Ok(imported) => {
                if !imported.unpaired.is_empty() {
                    editor.say(format!(
                        "Unpaired, cut for some builds only: {}",
                        imported.unpaired.join(", ")
                    ));
                }
                editor.toast(format!(
                    "Imported {}, {}",
                    words::counted(imported.parts.len(), "part"),
                    words::counted(imported.unpaired.len(), "unpaired file")
                ));
            }
            Err(error) => editor.toast(error),
        }
        return;
    }
    let Some(path) = drops.iter().rev().find(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    }) else {
        if !drops.is_empty() {
            editor.toast(
                "Drop a .glb on a skeleton to add a library or replace its rig, or a folder of \
                 part meshes to add its parts",
            );
        }
        return;
    };
    match std::fs::read(path) {
        Ok(bytes) => {
            let name = path.file_stem().map_or("library".to_owned(), |stem| {
                stem.to_string_lossy().into_owned()
            });
            match library::draft(editor, key, bytes, path.display().to_string(), &name) {
                Ok(draft) => editor.views.skeleton.view_mut(key).library = Some(draft),
                Err(error) => editor.toast(error),
            }
        }
        Err(error) => editor.toast(format!("{}: {error}", path.display())),
    }
}

/// Writes the skeleton at `key` as `change` leaves it, as one History step labelled `label`.
pub fn change(
    editor: &mut Editor,
    key: &str,
    label: &str,
    change: impl FnOnce(&mut SkeletonDef) -> Result<(), String>,
) -> bool {
    let edit = ContentEdit::new(format!("{label} in {}", shown_path(key)));
    change_with(editor, key, edit, change)
}

/// Writes the skeleton at `key` as `change` leaves it, as part of `edit`.
pub fn change_with(
    editor: &mut Editor,
    key: &str,
    edit: ContentEdit,
    change: impl FnOnce(&mut SkeletonDef) -> Result<(), String>,
) -> bool {
    let Some(Ok(row)) = editor.read::<SkeletonDef>(key) else {
        return false;
    };
    let mut skeleton = (*row).clone();
    if let Err(refusal) = change(&mut skeleton) {
        editor.toast(format!("{}: {refusal}", shown_path(key)));
        return false;
    }
    if skeleton == *row {
        return false;
    }
    editor.write_content_as(edit, key, &skeleton)
}

/// Gives the clip `clip` the cues `cues`, in order of time: none takes its entry away.
pub fn set_cues(editor: &mut Editor, key: &str, clip: &ClipName, mut cues: Vec<Cue>) -> bool {
    cues.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    change(editor, key, &format!("Mark {clip}'s cues"), |skeleton| {
        if cues.is_empty() {
            skeleton.cues.remove(clip);
        } else {
            skeleton.cues.insert(clip.clone(), cues);
        }
        Ok(())
    })
}

/// Gives the clip `clip` the pace `pace`, merging with the pace set just before it as one drag does.
pub fn set_pace(editor: &mut Editor, key: &str, clip: &ClipName, pace: MetersPerSec) -> bool {
    let edit = ContentEdit::new(format!("Pace {clip} in {}", shown_path(key))).merging();
    change_with(editor, key, edit, |skeleton| {
        skeleton.paces.insert(clip.clone(), pace);
        Ok(())
    })
}

/// Lists the skeleton's hands as `hands`, in order: the first main, the second off.
pub fn set_hands(editor: &mut Editor, key: &str, hands: Vec<Hand>) -> bool {
    change(editor, key, "Change the hands", |skeleton| {
        skeleton.hands = hands;
        Ok(())
    })
}

/// Makes the joint `joint` a hand named `name`, after the hands there are.
pub fn make_hand(editor: &mut Editor, key: &str, joint: &str, name: &str) -> bool {
    change(editor, key, &format!("Make {joint} a hand"), |skeleton| {
        if skeleton.hands.iter().any(|hand| hand.joint.0 == joint) {
            return Err(format!("{joint} is a hand already"));
        }
        skeleton.hands.push(Hand {
            name: name.to_owned(),
            joint: JointName::new(joint),
            offset: LocalOffset::default(),
            turn: LocalTurn::default(),
        });
        Ok(())
    })
}

/// Makes the joint `joint` a root of the group `group`, which it starts when there is none.
pub fn group_at(editor: &mut Editor, key: &str, joint: &str, group: &str) -> bool {
    change(
        editor,
        key,
        &format!("Group {group} at {joint}"),
        |skeleton| {
            let roots = skeleton.groups.entry(GroupName::new(group)).or_default();
            if roots.iter().any(|root| root.0 == joint) {
                return Err(format!("{joint} roots {group} already"));
            }
            roots.push(JointName::new(joint));
            Ok(())
        },
    )
}

/// Sets the skeleton's groups to `groups`.
pub fn set_groups(
    editor: &mut Editor,
    key: &str,
    groups: BTreeMap<GroupName, Vec<JointName>>,
) -> bool {
    change(editor, key, "Change the groups", |skeleton| {
        skeleton.groups = groups;
        Ok(())
    })
}

/// Sets the skeleton's layers to `layers`, refused while a leg is not a chain down the rig.
pub fn set_layers(editor: &mut Editor, key: &str, layers: Vec<Layer>) -> bool {
    if let Some((_, _, built)) = built(editor, key)
        && let Some(refusal) = layers::refusals(&built.rig, &layers).into_iter().next()
    {
        editor.toast(format!("{}: {refusal}", shown_path(key)));
        return false;
    }
    change(editor, key, "Change the layers", |skeleton| {
        skeleton.layers = layers;
        Ok(())
    })
}
