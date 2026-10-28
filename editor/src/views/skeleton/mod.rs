//! The skeleton view, on five pages: rigs, sets, hands, body and layers. Every page works from the
//! skeleton's file as it stands, with our rig put together from the files it names and every clip
//! of the game folder that plays on it, so a skeleton is set up from nothing. The stage shows the
//! rig as a model wearing it, or while none loads as its joints, posed as the game poses them.

pub mod body;
pub mod grips;
pub mod hands;
pub mod inspect;
pub mod layers;
pub mod preview;
pub mod rigging;
pub mod rigs;
pub mod sets;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use engine::core::content::gltf::{self, ClipKeys, NodeTree, Rest, TreeNode};
use engine::core::content::{Content, NamedContent};
use engine::core::math::Pos;
use engine::core::render::camera::ground_pos;
use engine::core::render::world::rig_clip;
use engine::core::units::{Meters, MetersPerSec};
use engine::systems::actor::ModelDef;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::{
    Clip, ClipCues, ClipPace, Cue, GroupName, Hand, JointName, Layer, SkeletonDef, SkeletonId,
};
use engine::systems::item::{ItemDef, Wear};
use serde::Deserialize;
use ui::egui::{self, Pos2, Rect, Response, Stroke, Ui, Vec2};
use ui::{chip, controls, glyph, theme};

use self::rigging::{GltfCache, Playable, RiggedFrom, Rigging};
use super::area::camera::AreaCanvas;
use super::area::viewport::StageClip;
use super::model::{self, hitbox::HitboxHandles};
use super::stage;
use crate::app::Editor;
use crate::project::{ContentEdit, ProjectFiles};
use crate::shell::sidebar;
use crate::tabs::shown_path;

#[derive(Default)]
pub struct SkeletonViews {
    pub views: HashMap<String, SkeletonView>,
    pub gltfs: GltfCache,
    rigged: HashMap<String, (RiggedFrom, Arc<Rigging>)>,
    posed: HashMap<(usize, StageKey), Posed>,
}

/// The keys a clip plays on a rig, with the rig, which the entry holds so no other stands where it
/// stood.
struct Posed {
    rig: Arc<NodeTree>,
    keys: Option<Arc<ClipKeys>>,
}

/// A stage clip as a key: what it plays and through which pairs.
type StageKey = (String, usize, Option<Vec<(String, String)>>);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkeletonPage {
    #[default]
    Rigs,
    Sets,
    Hands,
    Body,
    Layers,
}

impl SkeletonPage {
    const ALL: [SkeletonPage; 5] = [
        SkeletonPage::Rigs,
        SkeletonPage::Sets,
        SkeletonPage::Hands,
        SkeletonPage::Body,
        SkeletonPage::Layers,
    ];

    fn label(self) -> &'static str {
        match self {
            SkeletonPage::Rigs => "rigs",
            SkeletonPage::Sets => "sets",
            SkeletonPage::Hands => "hands",
            SkeletonPage::Body => "body",
            SkeletonPage::Layers => "layers",
        }
    }
}

pub struct SkeletonView {
    pub page: SkeletonPage,
    pub preview: preview::SkeletonPreview,
    /// The hand the hands page edits, by its place in the list.
    pub hand: usize,
    pub rigs: rigs::RigsState,
    pub sets: sets::SetsState,
    pub hands: hands::HandsState,
    pub layers: layers::LayersState,
    /// What is picked on the stage, for the inspector to set by number.
    pub chosen: Option<Chosen>,
    pub hitbox: HitboxHandles,
    pub grips: grips::Grips,
}

impl SkeletonView {
    /// Whether the page in front takes the next click on a joint of the stage.
    fn picks_joints(&self) -> bool {
        match self.page {
            SkeletonPage::Rigs => self.rigs.pairing.is_some(),
            SkeletonPage::Hands => self.hands.picking.is_some(),
            SkeletonPage::Layers => self.layers.picking,
            SkeletonPage::Sets | SkeletonPage::Body => false,
        }
    }
}

/// A thing on the stage the inspector sets by number once it is clicked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chosen {
    /// The hitbox of the model wearing the rig.
    Hitbox,
    /// The grip frame of the hand at this place in the list.
    Grip(usize),
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
    let Some(def) = super::read_shown::<SkeletonDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    let Some(rigging) = rigging(editor, key, &def) else {
        return;
    };
    let page = editor.views.skeleton.view(key).page;
    sidebar::inspector(editor, ui, "skeleton", |editor, ui| {
        inspect::show(editor, ui, key, &def, &rigging, page);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(editor, ui, key, page);
            ui.add_space(4.0);
            match page {
                SkeletonPage::Rigs => rigs::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Sets => sets::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Hands => hands::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Body => body::show(editor, ui, key, &def, &rigging),
                SkeletonPage::Layers => layers::show(editor, ui, key, &def, &rigging),
            }
        });
}

fn page_bar(editor: &mut Editor, ui: &mut Ui, key: &str, page: SkeletonPage) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        for other in SkeletonPage::ALL {
            if chip::mode(ui, other.label(), page == other).clicked() {
                editor.views.skeleton.view_mut(key).page = other;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggles = &mut editor.views.toggles;
            stage::toggle(ui, "turning", &mut toggles.turning);
            stage::toggle(ui, "skeleton", &mut toggles.skeleton);
            stage::toggle(ui, "grips", &mut toggles.grips);
            stage::toggle(ui, "hitbox", &mut toggles.hitbox);
        });
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
            preview: preview::SkeletonPreview::default(),
            hand: 0,
            rigs: rigs::RigsState::default(),
            sets: sets::SetsState::default(),
            hands: hands::HandsState::default(),
            layers: layers::LayersState::default(),
            chosen: None,
            hitbox: HitboxHandles::default(),
            grips: grips::Grips::default(),
        },
    );
}

/// Our rig and the clips that play on it for the skeleton at `key` as its file reads `def`, worked
/// out again only when the files or what it names of them change.
pub fn rigging(editor: &mut Editor, key: &str, def: &SkeletonDef) -> Option<Arc<Rigging>> {
    let Editor { project, views, .. } = editor;
    let files = &project.as_ref()?.files;
    let from = RiggedFrom {
        revision: files.revision(),
        rig: def.rig.clone(),
        sources: def.sources.clone(),
    };
    let views = &mut views.skeleton;
    if let Some((held, rigged)) = views.rigged.get(key)
        && *held == from
    {
        return Some(rigged.clone());
    }
    let rigged = Arc::new(rigging::rig_up(def, files, &mut views.gltfs));
    views.rigged.insert(key.to_owned(), (from, rigged.clone()));
    Some(rigged)
}

/// The skeleton at `key` as the game built it from the files as they last loaded, when it loaded.
fn skeleton_id(content: &Content, key: &str) -> Option<SkeletonId> {
    content.by_path::<SkeletonDef>(key)
}

/// The models that wear the skeleton at `key`, by their paths, with how each was built if it
/// loaded.
pub fn wearers<'c>(content: &'c Content, key: &str) -> Vec<(String, Option<&'c ActorModel>)> {
    let Some(id) = skeleton_id(content, key) else {
        return Vec::new();
    };
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

/// A combo labelled `label` of every clip that plays on the rig, filtered by any part of its file's
/// path as it is typed, offering no clip too as `none` names it.
pub fn clip_picker(
    ui: &mut Ui,
    label: &str,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    rigging: &Rigging,
    clip: &mut Option<Clip>,
    none: Option<&str>,
) {
    let filter_id = ui.make_persistent_id((&id, "filter"));
    ui.horizontal(|ui| {
        ui.label(theme::muted(label));
        controls::combo(id)
            .selected_text(
                clip.as_ref()
                    .map_or(none.unwrap_or("pick one").to_owned(), |clip| {
                        let length = rigging.playable(clip).map_or(0.0, |found| found.length.0);
                        format!("{} · {length:.2} s", clip.shown())
                    }),
            )
            .width(260.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                let mut filter: String = ui
                    .data_mut(|data| data.get_temp(filter_id))
                    .unwrap_or_default();
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .hint_text("filter by path")
                        .desired_width(240.0),
                )
                .request_focus();
                ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
                let mut picked = false;
                if let Some(none) = none {
                    picked |= ui.selectable_value(clip, None, none).clicked();
                }
                for found in matching(rigging, &filter).take(MAX_LISTED) {
                    picked |= ui
                        .selectable_value(
                            clip,
                            Some(found.clip.clone()),
                            format!("{} · {:.2} s", found.clip, found.length.0),
                        )
                        .clicked();
                }
                if picked {
                    ui.close();
                }
            });
    });
}

/// The most clips a list offers at once; typing narrows it.
pub const MAX_LISTED: usize = 200;

/// The clips that play on the rig whose file's path holds `filter`, ignoring case.
pub fn matching<'r>(rigging: &'r Rigging, filter: &str) -> impl Iterator<Item = &'r Playable> {
    let filter = filter.trim().to_lowercase();
    rigging.clips.iter().filter(move |found| {
        filter.is_empty() || found.clip.to_string().to_lowercase().contains(&filter)
    })
}

/// A stage drawn this frame: where, the canvas to draw over it through, and every joint of our rig
/// where it shows on the screen.
pub struct Staged {
    pub rect: Rect,
    pub response: Response,
    pub canvas: AreaCanvas,
    pub joints: Vec<(String, Pos2)>,
    lines: Vec<(Pos2, Pos2)>,
    /// Every node of the rig as it stands, in the rig's own frame.
    worlds: Vec<(String, Mat4)>,
    /// How the rig's frame stands in the scene.
    placed: Mat4,
    /// Whether this frame's click picked the hitbox or a grip frame, so it picks no joint.
    claimed: bool,
}

impl Staged {
    /// Draws the rig's joints over the stage, `lit` ones marked, and gives the joint clicked.
    pub fn pick(&self, ui: &Ui, lit: &[&str]) -> Option<String> {
        let painter = ui.painter_at(self.rect);
        for (from, to) in &self.lines {
            painter.line_segment([*from, *to], Stroke::new(1.5, theme::FAINT));
        }
        let hovered = self.response.hover_pos().and_then(|at| self.nearest(at));
        for (name, at) in &self.joints {
            let lit = lit.contains(&name.as_str());
            let colour = if lit { theme::ACCENT } else { theme::TEXT };
            painter.circle_filled(*at, if lit { 4.5 } else { 3.0 }, colour);
            if hovered.as_deref() == Some(name.as_str()) {
                painter.circle_stroke(*at, 7.0, Stroke::new(1.5, theme::ACCENT));
                glyph::haloed(
                    &painter,
                    *at + Vec2::new(9.0, -9.0),
                    egui::Align2::LEFT_BOTTOM,
                    name,
                    egui::FontId::monospace(12.0),
                    egui::Color32::WHITE,
                    glyph::Halo {
                        color: egui::Color32::BLACK,
                        width: 1.0,
                    },
                );
            }
        }
        if self.response.clicked() && !self.claimed {
            return self
                .response
                .interact_pointer_pos()
                .and_then(|at| self.nearest(at));
        }
        None
    }

    /// Draws the rig as its joints and the lines between them, in `colour`.
    pub fn draw(&self, ui: &Ui, colour: egui::Color32) {
        let painter = ui.painter_at(self.rect);
        for (from, to) in &self.lines {
            painter.line_segment([*from, *to], Stroke::new(2.0, colour));
        }
        for (_, at) in &self.joints {
            painter.circle_filled(*at, 2.5, colour);
        }
    }

    /// How the joint `name` stands, in the rig's frame.
    pub fn joint(&self, name: &str) -> Option<Mat4> {
        self.worlds
            .iter()
            .find(|(joint, _)| joint == name)
            .map(|(_, world)| *world)
    }

    /// Where `at`, a point in the rig's frame, shows on the screen.
    pub fn project(&self, at: Vec3) -> Pos2 {
        project(&self.canvas, self.placed.transform_point3(at))
    }

    /// The ground the rig stands on.
    pub fn ground(&self) -> Pos<Meters> {
        ground_pos(self.placed.w_axis.truncate())
    }

    fn nearest(&self, at: Pos2) -> Option<String> {
        self.joints
            .iter()
            .map(|(name, joint)| (name, joint.distance(at)))
            .filter(|(_, distance)| *distance < PICK_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(name, _)| name.clone())
    }
}

/// How near the pointer, in points, takes a joint.
const PICK_REACH: f32 = 10.0;

/// The hitbox of the model at `wearer` round the rig on `staged`, and the hands' grip frames on
/// it, as the view shows them: a click on either picks it for the inspector, and a click on neither
/// lets go of what was picked, unless it lands on a joint the page is picking. Gives whether the
/// pointer is dragging one, so the stage keeps still.
fn handles(
    editor: &mut Editor,
    ui: &Ui,
    key: &str,
    def: &SkeletonDef,
    staged: &mut Staged,
    wearer: Option<&str>,
) -> bool {
    let toggles = &editor.views.toggles;
    let (hitbox_shown, grips_shown) = (toggles.hitbox, toggles.grips);
    let view = editor.views.skeleton.view_mut(key);
    if !hitbox_shown {
        view.hitbox.let_go();
    }
    view.chosen = view.chosen.filter(|chosen| match chosen {
        Chosen::Hitbox => hitbox_shown,
        Chosen::Grip(_) => grips_shown,
    });
    let chosen = view.chosen;
    let on_joint = view.picks_joints()
        && staged.response.clicked()
        && staged
            .response
            .interact_pointer_pos()
            .is_some_and(|at| staged.nearest(at).is_some());
    let hitbox = wearer.filter(|_| hitbox_shown).and_then(|path| {
        let model = editor.read::<ModelDef>(path)?.ok()?;
        Some((path.to_owned(), model.hitbox))
    });
    let view = editor.views.skeleton.view_mut(key);
    let touched = match &hitbox {
        Some((_, capsule)) => model::hitbox::overlay(
            ui,
            &staged.canvas,
            &staged.response,
            staged.ground(),
            *capsule,
            chosen == Some(Chosen::Hitbox),
            &mut view.hitbox,
        ),
        None => Default::default(),
    };
    let grip = match chosen {
        Some(Chosen::Grip(index)) => Some(index),
        _ => None,
    };
    let hands = view.grips.working(&def.hands);
    let gripped = grips::overlay(editor, ui, key, staged, &hands, grip);
    let view = editor.views.skeleton.view_mut(key);
    view.chosen = match (gripped.picked, touched.picked) {
        _ if on_joint => view.chosen,
        (Some(index), _) => {
            view.hand = index;
            Some(Chosen::Grip(index))
        }
        (None, true) => Some(Chosen::Hitbox),
        (None, false) if staged.response.clicked() && !gripped.claims_click => None,
        (None, false) => view.chosen,
    };
    staged.claimed =
        !on_joint && (gripped.picked.is_some() || touched.picked || gripped.claims_click);
    let busy = view.hitbox.dragging();
    if let (Some(sized), Some((path, _))) = (touched.sized, hitbox) {
        model::set_hitbox(editor, &path, sized);
    }
    busy
}

/// Where each joint of `rig` shows on the stage's picture, standing `pose` (at rest without one)
/// as `placed` stands its frame in the scene.
fn place(
    rect: Rect,
    response: Response,
    canvas: AreaCanvas,
    rig: &NodeTree,
    pose: Option<&[Rest]>,
    placed: Mat4,
) -> Staged {
    let worlds = posed_worlds(rig, pose);
    let screen = |world: &Mat4| project(&canvas, placed.transform_point3(world.w_axis.truncate()));
    let joints: Vec<(String, Pos2)> = rig
        .joints()
        .map(|joint| (rig.nodes[joint].name.clone(), screen(&worlds[joint])))
        .collect();
    let lines = rig
        .joints()
        .filter_map(|joint| {
            let parent = rig.nodes[joint]
                .parent
                .filter(|&parent| !rig.nodes[parent].mesh)?;
            Some((screen(&worlds[parent]), screen(&worlds[joint])))
        })
        .collect();
    let worlds = rig
        .nodes
        .iter()
        .zip(worlds)
        .map(|(node, world)| (node.name.clone(), world))
        .collect();
    Staged {
        rect,
        response,
        canvas,
        joints,
        lines,
        worlds,
        placed,
        claimed: false,
    }
}

/// Where `at`, a point in the scene, shows on `canvas`.
fn project(canvas: &AreaCanvas, at: Vec3) -> Pos2 {
    let ground = ground_pos(at);
    let lift = at.y - canvas.area().heights.height_at(ground).0;
    canvas.lifted(ground, Meters(lift))
}

/// Every node of `tree` in the scene, standing `pose`, or at rest without one.
pub fn posed_worlds(tree: &NodeTree, pose: Option<&[Rest]>) -> Vec<Mat4> {
    match pose {
        None => tree.worlds(),
        Some(pose) => NodeTree {
            nodes: tree
                .nodes
                .iter()
                .zip(pose)
                .map(|(node, rest)| TreeNode {
                    rest: *rest,
                    ..node.clone()
                })
                .collect(),
        }
        .worlds(),
    }
}

/// How `rig` stands `at` seconds into `clip`, as the game plays it there.
fn pose_of(
    editor: &mut Editor,
    rig: &Arc<NodeTree>,
    clip: &StageClip,
    at: f32,
) -> Option<Vec<Rest>> {
    let keys = rig_keys(editor, rig, clip)?;
    Some(keys.pose(rig, at))
}

/// The keys `clip` plays on `rig`, made once and kept while the rig stands.
fn rig_keys(editor: &mut Editor, rig: &Arc<NodeTree>, clip: &StageClip) -> Option<Arc<ClipKeys>> {
    let Editor { project, views, .. } = editor;
    let files = &project.as_ref()?.files;
    let views = &mut views.skeleton;
    let key = (
        Arc::as_ptr(rig) as usize,
        (clip.file.clone(), clip.animation, clip.pairs.clone()),
    );
    views
        .posed
        .retain(|_, posed| Arc::strong_count(&posed.rig) > 1);
    if let Some(posed) = views.posed.get(&key) {
        return posed.keys.clone();
    }
    if views.posed.len() > MAX_POSED {
        views.posed.clear();
    }
    let keys = made_keys(files, &mut views.gltfs, rig, clip).map(Arc::new);
    views.posed.insert(
        key,
        Posed {
            rig: rig.clone(),
            keys: keys.clone(),
        },
    );
    keys
}

/// The most clips kept posed at once.
const MAX_POSED: usize = 64;

fn made_keys(
    files: &ProjectFiles,
    gltfs: &mut GltfCache,
    rig: &Arc<NodeTree>,
    clip: &StageClip,
) -> Option<ClipKeys> {
    let info = gltfs.info(files, &clip.file)?;
    let keys = gltf::clip_keys(files.get(&clip.file)?, clip.animation).ok()?;
    let make = rig_clip::maker(rig.clone(), clip.pairs.clone());
    Some(make(&info.tree, &keys))
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
pub fn set_cues(editor: &mut Editor, key: &str, clip: &Clip, mut cues: Vec<Cue>) -> bool {
    cues.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    change(
        editor,
        key,
        &format!("Mark {}'s cues", clip.shown()),
        |skeleton| {
            skeleton.cues.retain(|held| held.clip != *clip);
            if !cues.is_empty() {
                skeleton.cues.push(ClipCues {
                    clip: clip.clone(),
                    cues,
                });
                skeleton.cues.sort_by(|a, b| a.clip.cmp(&b.clip));
            }
            Ok(())
        },
    )
}

/// Gives the clip `clip` the pace `pace`, merging with the pace set just before it as one drag does.
pub fn set_pace(editor: &mut Editor, key: &str, clip: &Clip, pace: MetersPerSec) -> bool {
    let edit = ContentEdit::new(format!("Pace {} in {}", clip.shown(), shown_path(key))).merging();
    change_with(editor, key, edit, |skeleton| {
        match skeleton.paces.iter_mut().find(|held| held.clip == *clip) {
            Some(held) => held.pace = pace,
            None => {
                skeleton.paces.push(ClipPace {
                    clip: clip.clone(),
                    pace,
                });
                skeleton.paces.sort_by(|a, b| a.clip.cmp(&b.clip));
            }
        }
        Ok(())
    })
}

/// Changes the hand at `index` to `hand`, merging with the change made just before it as one drag
/// does.
pub fn set_hand(editor: &mut Editor, key: &str, index: usize, hand: Hand) -> bool {
    let edit =
        ContentEdit::new(format!("Change hand {} in {}", hand.name, shown_path(key))).merging();
    change_with(editor, key, edit, |skeleton| {
        let held = skeleton
            .hands
            .get_mut(index)
            .ok_or_else(|| format!("it has no hand {}", index + 1))?;
        *held = hand;
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
    let rig = editor
        .read::<SkeletonDef>(key)
        .and_then(Result::ok)
        .and_then(|def| rigging(editor, key, &def))
        .and_then(|rigging| rigging.rig.clone());
    if let Some(rig) = rig
        && let Some(refusal) = layers::refusals(&rig, &layers).into_iter().next()
    {
        editor.toast(format!("{}: {refusal}", shown_path(key)));
        return false;
    }
    change(editor, key, "Change the layers", |skeleton| {
        skeleton.layers = layers;
        Ok(())
    })
}
