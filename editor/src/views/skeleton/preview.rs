//! The stage every page of the skeleton view shows, drawn by this one module on each: the rig worn
//! by a model of the game, or drawn as its joints while none loads, playing an action as the game
//! plays it or any clip that plays on the rig, holding an item in each hand and dressed as the body
//! page dresses it. A body in an action that carries it over the ground walks a loop round the
//! stage's middle with the camera following, its skeleton's layers running as the client runs
//! them. The hitbox, the grip frames and the joints are drawn over it as the editor's toggles say.

use std::collections::BTreeMap;
use std::f32::consts::TAU;

use bevy::prelude::{Quat, Transform};
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::render::camera::{ground_pos, world_point};
use engine::core::time::Millis;
use engine::core::units::Meters;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::{Clip, EquipmentSlot, SkeletonDef};
use engine::systems::actor::stance::{self, HandIndex};
use engine::systems::actor::{Action, Appearance, ModelDef, Motion, Pose, appearance};
use engine::systems::item::{ItemDef, ItemId};
use serde::Deserialize;
use strum::VariantArray;
use ui::egui::{Ui, Vec2};
use ui::{controls, theme};

use super::super::area::viewport::StageLayers;
use super::super::cues::{self, Scrub};
use super::super::stage::{self, StageState};
use super::rigging::{Playable, Rigging};
use super::{Staged, body, clip_picker, hand_items, handles, place, pose_of, wearers};
use crate::app::Editor;
use crate::tabs::shown_path;

/// What a skeleton view's stage shows, the same on every page.
pub struct SkeletonPreview {
    /// The model the stage shows the rig as, by its file's path.
    pub model: Option<String>,
    pub plays: PreviewPlays,
    pub action: Action,
    /// The clip the stage plays while it plays clips; none stands the rig at rest.
    pub clip: Option<Clip>,
    /// The item each hand holds, by its file's path.
    pub held: BTreeMap<HandIndex, String>,
    /// The items the model wears, by the slot each is worn in and its file's path.
    pub wearing: BTreeMap<EquipmentSlot, String>,
    pub scrub: Scrub,
    pub stage: StageState,
    /// The clip played last frame, so the playhead starts over when another takes its place.
    played: Option<Clip>,
    /// How far round the loop a body walking it has come.
    walked: Meters,
}

impl Default for SkeletonPreview {
    fn default() -> SkeletonPreview {
        SkeletonPreview {
            model: None,
            plays: PreviewPlays::default(),
            action: Action::Idle,
            clip: None,
            held: BTreeMap::new(),
            wearing: BTreeMap::new(),
            scrub: Scrub::default(),
            stage: StageState::framing(1.3),
            played: None,
            walked: Meters(0.0),
        }
    }
}

impl SkeletonPreview {
    /// Plays `clip` from its start.
    pub fn play(&mut self, clip: Clip) {
        self.plays = PreviewPlays::Clip;
        self.clip = Some(clip);
        self.scrub.restart();
    }
}

/// What the stage plays: an action, as the model's sets name its clip for what it holds, or any
/// clip that plays on the rig.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewPlays {
    #[default]
    Action,
    Clip,
}

/// What the preview showed this frame: its stage, the clip playing there and its playhead, and the
/// joint clicked while the page picks one.
pub struct Previewed {
    pub staged: Staged,
    pub playing: Option<Playable>,
    pub playhead: Millis,
    pub picked: Option<String>,
}

/// How far from the middle of the stage a body walks its loop.
const LOOP: f32 = 1.2;

/// The preview of the skeleton at `key` as its file reads `def`: its pickers, then its stage in
/// the room left above `below` points, the joints `lit` marked.
pub fn show(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    def: &SkeletonDef,
    rigging: &Rigging,
    lit: &[&str],
    below: f32,
) -> Option<Previewed> {
    let rig = rigging.rig.clone()?;
    let content = editor.loaded()?;
    pickers(editor, ui, key, def, rigging, &content);
    let model = wearer(editor, key, &content);
    let held = held_items(editor, key, &content, def.hands.len());
    let preview = &editor.views.skeleton.view(key).preview;
    let action = (preview.plays == PreviewPlays::Action).then_some(preview.action);
    let clip = match action {
        Some(action) => def
            .named(
                model.as_ref().and_then(|(_, model)| model.build()),
                stance::of_hands(&content, &held),
                &Motion::Action(action),
                Pose::Idle,
            )
            .map(|(cell, _)| cell.nth(0).clip().clone()),
        None => preview.clip.clone(),
    };
    let playing = clip
        .as_ref()
        .and_then(|clip| rigging.playable(clip))
        .cloned();
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let preview = &mut editor.views.skeleton.view_mut(key).preview;
    if preview.played != clip {
        preview.played = clip;
        preview.scrub.restart();
    }
    cues::advance(
        ui,
        &mut preview.scrub,
        playing.as_ref().map(|found| found.length.millis()),
    );
    let playhead = preview.scrub.playhead;
    let crossed = playing.as_ref().map_or_else(Vec::new, |found| {
        preview
            .scrub
            .crossed(def.cues_of(&found.clip), found.length.millis())
    });
    let pace = playing
        .as_ref()
        .filter(|_| action.is_some_and(Action::moves))
        .and_then(|found| def.pace(&found.clip));
    if let Some(pace) = pace
        && preview.scrub.playing
    {
        preview.walked = Meters((preview.walked.0 + pace.0.0 * dt) % (TAU * LOOP));
        ui.ctx().request_repaint();
    }
    let placed = around(preview.walked.0 / LOOP, pace.is_some());
    preview.stage.following = ground_pos(placed.translation) - stage::center();
    let stage_clip = playing.as_ref().map(|found| found.stage_clip(def));
    let pose = stage_clip
        .as_ref()
        .and_then(|clip| pose_of(editor, &rig, clip, playhead.seconds().0));
    let size = Vec2::new(
        ui.available_width(),
        (ui.available_height() - below).max(140.0),
    );
    let mut state = std::mem::take(&mut editor.views.skeleton.view_mut(key).preview.stage);
    let turntable = stage::turntable(editor, ui, &format!("skeleton:{key}"), size, &mut state);
    editor.views.skeleton.view_mut(key).preview.stage = state;
    let turntable = turntable?;
    let mut staged = place(
        turntable.rect,
        turntable.response.clone(),
        turntable.canvas.clone(),
        &rig,
        pose.as_deref(),
        placed.to_matrix(),
    );
    let model_path = model.as_ref().map(|(path, _)| path.clone());
    if handles(editor, ui, key, def, &mut staged, model_path.as_deref()) {
        editor.views.toggles.turning = false;
    }
    let picked = joints(editor, ui, key, &staged, lit, model.is_some());
    let stepped = crossed.iter().any(|cue| cue.step);
    let mut cast = Vec::new();
    if let Some((path, built)) = model
        && let Some(id) = content.by_path::<ModelDef>(&path)
    {
        let body = built.dressed(
            &content,
            &appearance::look(
                &content,
                &Appearance::of(id),
                &body::wearing_items(editor, key, &content),
            ),
        );
        let hands = editor.views.skeleton.view(key).grips.working(&def.hands);
        let props = stage::props(&content, built, &hands, &held);
        let holds = !props.is_empty();
        let mut standing = stage::standing(
            body,
            stage_clip,
            playing.as_ref().map(|_| playhead.seconds()),
            crossed,
            props,
        );
        standing.transform = placed;
        standing.layers = match action {
            Some(action) => Some(StageLayers {
                model: id,
                action,
                stepped,
            }),
            None => holds.then(|| stage::holding(id)),
        };
        cast.push(standing);
    }
    turntable.stand(editor, ui, cast);
    Some(Previewed {
        staged,
        playing,
        playhead,
        picked,
    })
}

/// The model the stage of the view at `key` shows the rig as: the one picked, else the first that
/// loaded.
pub fn wearer<'c>(
    editor: &Editor,
    key: &str,
    content: &'c Content,
) -> Option<(String, &'c ActorModel)> {
    let picked = editor.views.skeleton.view(key).preview.model.clone();
    let wearers = wearers(content, key);
    picked
        .and_then(|picked| {
            wearers
                .iter()
                .find(|(path, model)| *path == picked && model.is_some())
        })
        .or_else(|| wearers.iter().find(|(_, model)| model.is_some()))
        .and_then(|(path, model)| Some((path.clone(), (*model)?)))
}

/// The pickers over the stage: the model wearing the rig, whether it plays an action or a clip and
/// which, and the item in each hand.
fn pickers(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    def: &SkeletonDef,
    rigging: &Rigging,
    content: &Content,
) {
    let mut model = wearer(editor, key, content).map(|(path, _)| path);
    let preview = &editor.views.skeleton.view(key).preview;
    let mut plays = preview.plays;
    let mut action = preview.action;
    let mut clip = preview.clip.clone();
    let mut held = preview.held.clone();
    ui.horizontal_wrapped(|ui| {
        model_picker(ui, key, content, &mut model);
        ui.add_space(8.0);
        ui.label(theme::muted("preview"));
        controls::combo(("skeleton-plays", key))
            .selected_text(plays_label(plays))
            .width(80.0)
            .show_ui(ui, |ui| {
                for other in [PreviewPlays::Action, PreviewPlays::Clip] {
                    ui.selectable_value(&mut plays, other, plays_label(other));
                }
            });
        match plays {
            PreviewPlays::Action => {
                controls::combo(("skeleton-action", key))
                    .selected_text(action.as_ref())
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for other in Action::VARIANTS {
                            ui.selectable_value(&mut action, *other, other.as_ref());
                        }
                    });
            }
            PreviewPlays::Clip => {
                clip_picker(
                    ui,
                    "",
                    ("skeleton-clip", key),
                    rigging,
                    &mut clip,
                    Some("rest"),
                );
            }
        }
    });
    if !def.hands.is_empty() {
        let items = hand_items(content);
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::muted("item"));
            for (index, hand) in def.hands.iter().enumerate() {
                let at = HandIndex(index as u8);
                let mut item = held.get(&at).cloned();
                let shown = item
                    .as_ref()
                    .and_then(|path| items.iter().find(|(held, _)| held == path))
                    .map_or("nothing", |(_, name)| name.as_str());
                controls::combo(("skeleton-held", key, index))
                    .selected_text(format!("{}: {shown}", hand.name))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut item, None, "nothing");
                        for (path, name) in &items {
                            ui.selectable_value(&mut item, Some(path.clone()), name);
                        }
                    });
                match item {
                    Some(path) => held.insert(at, path),
                    None => held.remove(&at),
                };
            }
        });
    }
    let preview = &mut editor.views.skeleton.view_mut(key).preview;
    if model.is_some() {
        preview.model = model;
    }
    preview.plays = plays;
    preview.action = action;
    preview.clip = clip;
    preview.held = held;
}

fn plays_label(plays: PreviewPlays) -> &'static str {
    match plays {
        PreviewPlays::Action => "action",
        PreviewPlays::Clip => "clip",
    }
}

/// Which model wears the rig on the stage.
fn model_picker(ui: &mut Ui, key: &str, content: &Content, picked: &mut Option<String>) {
    let wearers = wearers(content, key);
    ui.label(theme::muted("model"));
    controls::combo(("skeleton-wearer", key))
        .selected_text(
            picked
                .as_deref()
                .map_or("its joints".to_owned(), |path| shown_path(path).to_owned()),
        )
        .width(180.0)
        .show_ui(ui, |ui| {
            for (path, model) in &wearers {
                let label = if model.is_some() {
                    shown_path(path).to_owned()
                } else {
                    format!("{} · did not load", shown_path(path))
                };
                ui.add_enabled_ui(model.is_some(), |ui| {
                    ui.selectable_value(picked, Some(path.clone()), label);
                });
            }
        });
}

/// The items the stage's hands hold, among the `hands` the skeleton has.
fn held_items(
    editor: &Editor,
    key: &str,
    content: &Content,
    hands: usize,
) -> BTreeMap<HandIndex, ItemId> {
    editor
        .views
        .skeleton
        .view(key)
        .preview
        .held
        .iter()
        .filter(|(hand, _)| hand.index() < hands)
        .filter_map(|(hand, path)| Some((*hand, content.by_path::<ItemDef>(path)?)))
        .collect()
}

/// The rig's joints over the stage as the toggles ask, always while no model wears the rig or the
/// page picks one, `lit` ones marked; gives the joint clicked while the page picks one.
fn joints(
    editor: &Editor,
    ui: &Ui,
    key: &str,
    staged: &Staged,
    lit: &[&str],
    worn: bool,
) -> Option<String> {
    let picks = editor.views.skeleton.view(key).picks_joints();
    if !worn {
        staged.draw(ui, theme::TEXT);
    }
    if !(picks || editor.views.toggles.skeleton) {
        return None;
    }
    staged.pick(ui, lit).filter(|_| picks)
}

/// Where a body stands `turned` radians round the loop, facing along it when it walks.
fn around(turned: f32, walking: bool) -> Transform {
    let center = stage::center();
    if !walking {
        return Transform::from_translation(world_point(center, Meters(0.0)));
    }
    let at = Pos::new(
        center.x + LOOP * turned.cos(),
        center.y + LOOP * turned.sin(),
    );
    Transform {
        translation: world_point(at, Meters(0.0)),
        rotation: Quat::from_rotation_y(-turned),
        ..Transform::default()
    }
}
