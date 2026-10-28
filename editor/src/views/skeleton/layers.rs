//! The layers page: the skeleton's groups, and the layers that move its bodies beyond their clips,
//! each with its fields; the stage walks a body wearing the skeleton round a loop with its layers
//! running as the client runs them, Reduce motion to toggle, or stands the rig at rest for the
//! joint new roots and layers start from to be clicked. A leg that is not a chain down the tree is
//! refused with the joint named, as the loader refuses it.

use std::collections::BTreeMap;

use bevy::prelude::{Quat, Transform};
use engine::core::content::gltf::NodeTree;
use engine::core::geometry::Radians;
use engine::core::math::Pos;
use engine::core::render::camera::world_point;
use engine::core::render::world::layers::Gait;
use engine::core::time::{Hertz, Millis, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::{Cue, GroupName, JointName, Layer, Leg, SkeletonDef};
use engine::systems::actor::{Action, ModelDef, Motion, Pose};
use strum::VariantArray;
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{chip, controls, form, panel, theme};

use super::super::area::viewport::{StageClip, StageLayers};
use super::rigging::Rigging;
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;
use crate::views::stage;

/// The actions a layer may run during: never a hit or a death, which play their clip alone.
const CALM: [Action; 5] = [
    Action::Idle,
    Action::Walk,
    Action::Run,
    Action::Sprint,
    Action::Talk,
];
/// How far from the middle of the stage the body walks its loop.
const LOOP: f32 = 1.2;

/// What the layers page keeps: the chosen layer, the joint new entries start from, the action the
/// stage shows, and the stage framing the whole loop.
pub struct LayersState {
    pub layer: usize,
    pub joint: Option<String>,
    pub action: Option<Action>,
    pub new_group: String,
    /// Whether the stage stands the rig at rest for the joint to be clicked, rather than walking.
    pub picking: bool,
    pub reduce_motion: bool,
    pub stage: stage::StageState,
}

impl Default for LayersState {
    fn default() -> LayersState {
        LayersState {
            layer: 0,
            joint: None,
            action: None,
            new_group: String::new(),
            picking: false,
            reduce_motion: false,
            stage: stage::StageState::framing(LOOP * 3.0, false),
        }
    }
}

/// Every leg of `layers` that is not a chain down `rig`, as the loader words it.
pub fn refusals(rig: &NodeTree, layers: &[Layer]) -> Vec<String> {
    let mut refusals = Vec::new();
    for layer in layers {
        for (_, joint) in layer.joints() {
            if rig.named(joint.as_str()).is_none() {
                refusals.push(format!("{joint} is not a joint of our rig"));
            }
        }
        let Layer::Feet { legs, .. } = layer else {
            continue;
        };
        for leg in legs {
            for pair in leg.joints.windows(2) {
                let (Some(upper), Some(lower)) =
                    (rig.named(pair[0].as_str()), rig.named(pair[1].as_str()))
                else {
                    continue;
                };
                if lower == upper || !rig.descends(lower, upper) {
                    refusals.push(format!(
                        "leg {:?}: {} is not a descendant of {}",
                        leg.name, pair[1], pair[0]
                    ));
                    break;
                }
            }
        }
    }
    refusals
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    let Some(rig) = rigging.rig.clone() else {
        form::hint(
            ui,
            "Pick our rig's files on the rigs page to give it groups and layers.",
        );
        return;
    };
    let width = ui.available_width();
    let list_width = (width * 0.4).clamp(260.0, 420.0);
    let [left, right] = panel::split(ui, list_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("layers", key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                groups(editor, ui, key, def, &rig);
                ui.add_space(10.0);
                layers(editor, ui, key, def, &rig);
            });
    });
    panel::body(ui, right, |ui| {
        if editor.views.skeleton.view(key).layers.picking {
            pick(editor, ui, key, def, rigging);
        } else {
            walk(editor, ui, key, def, &rig);
        }
    });
}

/// The rig at rest, its joints to click: the one clicked is where new roots and layers start.
fn pick(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rigging: &Rigging) {
    if ui.button("walk again").clicked() {
        editor.views.skeleton.view_mut(key).layers.picking = false;
        return;
    }
    editor.views.skeleton.view_mut(key).clip = None;
    let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
    let Some(staged) = super::stage_rig(editor, ui, key, def, rigging, size, Vec::new()) else {
        return;
    };
    let lit = editor.views.skeleton.view(key).layers.joint.clone();
    let lit: Vec<&str> = lit.as_deref().into_iter().collect();
    if let Some(clicked) = staged.pick(ui, &lit) {
        editor.views.skeleton.view_mut(key).layers.joint = Some(clicked);
    }
    form::hint(ui, "Click a joint: new roots and layers start from it.");
}

fn joint_names(rig: &NodeTree) -> Vec<String> {
    rig.joints()
        .map(|joint| rig.nodes[joint].name.clone())
        .collect()
}

/// The joint new roots, layers and legs start from: picked here, or clicked on the stage.
fn joint_picker(editor: &mut Editor, ui: &mut Ui, key: &str, rig: &NodeTree) -> Option<String> {
    let names = joint_names(rig);
    let view = editor.views.skeleton.view_mut(key);
    let mut joint = view
        .layers
        .joint
        .clone()
        .filter(|joint| names.contains(joint))
        .or_else(|| names.first().cloned());
    let mut picking = view.layers.picking;
    ui.horizontal(|ui| {
        ui.label(theme::muted("at joint"));
        controls::combo(("layers-joint", key))
            .selected_text(joint.clone().unwrap_or_default())
            .width(160.0)
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut joint, Some(name.clone()), name);
                }
            });
        ui.toggle_value(&mut picking, "click it on the stage");
    });
    let view = editor.views.skeleton.view_mut(key);
    view.layers.joint = joint.clone();
    view.layers.picking = picking;
    joint
}

fn groups(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rig: &NodeTree) {
    let groups = def.groups.clone();
    form::section_count(ui, "Groups", groups.len());
    let mut changed: Option<BTreeMap<GroupName, Vec<JointName>>> = None;
    for (group, roots) in &groups {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(group.as_str()).strong());
            for (at, root) in roots.iter().enumerate() {
                let (_, closed) = chip::closable(ui, root.as_str());
                if closed {
                    let mut fewer = groups.clone();
                    let left = fewer.get_mut(group).expect("the group listed");
                    left.remove(at);
                    if left.is_empty() {
                        fewer.remove(group);
                    }
                    changed = Some(fewer);
                }
            }
        });
    }
    let joint = joint_picker(editor, ui, key, rig);
    let view = editor.views.skeleton.view_mut(key);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut view.layers.new_group)
                .hint_text("group")
                .desired_width(120.0),
        );
        let name = view.layers.new_group.trim().to_owned();
        if ui
            .add_enabled(
                !name.is_empty() && joint.is_some(),
                egui::Button::new("+ Add root"),
            )
            .on_hover_text("the joint and everything under it join the group")
            .clicked()
            && let Some(joint) = &joint
        {
            let mut more = groups.clone();
            let roots = more.entry(GroupName::new(name)).or_default();
            if !roots.iter().any(|root| root.0 == *joint) {
                roots.push(JointName::new(joint.clone()));
            }
            changed = Some(more);
        }
    });
    form::hint(
        ui,
        "A group is the joints under its roots; a set may keep a clip to one, over the body's \
         resting clip. No joint is in two groups.",
    );
    if let Some(groups) = changed {
        super::set_groups(editor, key, groups);
    }
}

fn layers(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rig: &NodeTree) {
    let layers = def.layers.clone();
    let chosen = editor
        .views
        .skeleton
        .view(key)
        .layers
        .layer
        .min(layers.len().saturating_sub(1));
    form::section_count(ui, "Layers", layers.len());
    let mut pick = None;
    let mut removed = None;
    for (index, layer) in layers.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::selectable(index == chosen, summary(layer)))
                .clicked()
            {
                pick = Some(index);
            }
            if controls::remove(ui, "layer").clicked() {
                removed = Some(index);
            }
        });
    }
    let from = editor
        .views
        .skeleton
        .view(key)
        .layers
        .joint
        .clone()
        .or_else(|| joint_names(rig).first().cloned());
    let mut added = None;
    if let Some(from) = &from {
        ui.menu_button(format!("+ Add layer at {from}"), |ui| {
            for (label, layer) in fresh_layers(rig, from) {
                if ui.button(label).clicked() {
                    added = Some(layer);
                    ui.close();
                }
            }
        });
    }
    if let Some(index) = removed {
        let mut fewer = layers.clone();
        fewer.remove(index);
        set(editor, key, rig, fewer, "Remove a layer", false);
    }
    if let Some(layer) = added {
        let mut more = layers.clone();
        more.push(layer);
        if set(editor, key, rig, more, "Add a layer", false) {
            pick = Some(layers.len());
        }
    }
    if let Some(index) = pick {
        editor.views.skeleton.view_mut(key).layers.layer = index;
    }
    let Some(layer) = layers.get(chosen) else {
        form::hint(
            ui,
            "No layers: the bodies play their clips alone. Feet plant on the ground, a chain \
             springs behind, a wave sways down a spine, a squash keeps a blob's volume.",
        );
        return;
    };
    ui.add_space(8.0);
    form::rule(ui);
    let mut edited = layer.clone();
    fields(ui, key, rig, &from, &mut edited);
    for refusal in refusals(rig, std::slice::from_ref(&edited)) {
        ui.colored_label(theme::DANGER, refusal);
    }
    if edited != *layer {
        let mut changed = layers.clone();
        changed[chosen] = edited;
        set(editor, key, rig, changed, "Change a layer", true);
    }
}

/// Sets the layers when every leg is a chain down the rig, else says which joint is not.
fn set(
    editor: &mut Editor,
    key: &str,
    rig: &NodeTree,
    layers: Vec<Layer>,
    label: &str,
    merging: bool,
) -> bool {
    if let Some(refusal) = refusals(rig, &layers).into_iter().next() {
        editor.toast(format!("{}: {refusal}", shown_path(key)));
        return false;
    }
    let mut edit = ContentEdit::new(format!("{label} in {}", shown_path(key)));
    if merging {
        edit = edit.merging();
    }
    super::change_with(editor, key, edit, |skeleton| {
        skeleton.layers = layers;
        Ok(())
    })
}

fn summary(layer: &Layer) -> String {
    match layer {
        Layer::Feet { legs, gait, .. } => {
            format!("feet · {} legs · {}", legs.len(), gait.as_ref())
        }
        Layer::Chain { from, follow, .. } => {
            format!("chain · {from}{}", if *follow { " · follows" } else { "" })
        }
        Layer::Wave { from, .. } => format!("wave · {from}"),
        Layer::Squash { joint, .. } => format!("squash · {joint}"),
    }
}

/// A layer of each kind starting at `from`, as the add menu offers them.
fn fresh_layers(rig: &NodeTree, from: &str) -> Vec<(&'static str, Layer)> {
    let joint = JointName::new(from);
    let during = CALM.to_vec();
    vec![
        (
            "feet · a leg ending here",
            Layer::Feet {
                legs: vec![leg_to(rig, from, "leg 1")],
                gait: Gait::Lateral,
                stride: Meters(0.6),
                lift: Meters(0.1),
                during: vec![Action::Walk, Action::Run],
            },
        ),
        (
            "chain · springing from here",
            Layer::Chain {
                from: joint.clone(),
                follow: false,
                stiffness: 0.5,
                drag: 0.4,
                gravity: 0.0,
                during: during.clone(),
            },
        ),
        (
            "wave · swaying from here",
            Layer::Wave {
                from: joint.clone(),
                amplitude: Radians(0.3),
                hz: Hertz(1.0),
                lag: 1.0,
                during: during.clone(),
            },
        ),
        (
            "squash · this joint",
            Layer::Squash {
                joint,
                by_speed: 0.1,
                on_step: 0.2,
                ease: Seconds(0.15),
                during,
            },
        ),
    ]
}

/// A leg ending at `foot`: the foot and the two joints above it, from the top down.
fn leg_to(rig: &NodeTree, foot: &str, name: &str) -> Leg {
    let mut joints = Vec::new();
    let mut at = rig.named(foot);
    while let Some(node) = at
        && joints.len() < 3
    {
        if !rig.joints().any(|joint| joint == node) {
            break;
        }
        joints.push(JointName::new(rig.nodes[node].name.clone()));
        at = rig.nodes[node].parent;
    }
    joints.reverse();
    Leg {
        name: name.to_owned(),
        joints,
    }
}

/// The chosen layer's fields, edited in place.
fn fields(ui: &mut Ui, key: &str, rig: &NodeTree, from: &Option<String>, layer: &mut Layer) {
    ui.label(RichText::new(summary(layer)).strong());
    let names = joint_names(rig);
    let joint_field = |ui: &mut Ui, id: &str, joint: &mut JointName| {
        controls::combo((id.to_owned(), key))
            .selected_text(joint.as_str())
            .width(150.0)
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(joint, JointName::new(name.clone()), name);
                }
            });
    };
    egui::Grid::new(("layer-fields", key))
        .num_columns(2)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| match layer {
            Layer::Feet {
                legs,
                gait,
                stride,
                lift,
                ..
            } => {
                ui.label(theme::muted("gait"));
                controls::combo(("layer-gait", key))
                    .selected_text(gait.as_ref())
                    .show_ui(ui, |ui| {
                        for other in Gait::VARIANTS {
                            ui.selectable_value(gait, *other, other.as_ref());
                        }
                    });
                ui.end_row();
                number(ui, "stride · m", &mut stride.0, 0.01, 0.05..=4.0);
                number(ui, "lift · m", &mut lift.0, 0.005, 0.0..=1.0);
                ui.label(theme::muted("legs"));
                ui.vertical(|ui| {
                    let mut removed = None;
                    for (index, leg) in legs.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut leg.name).desired_width(80.0));
                            ui.label(theme::mono(
                                leg.joints
                                    .iter()
                                    .map(JointName::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" → "),
                            ));
                            if controls::remove(ui, "leg").clicked() {
                                removed = Some(index);
                            }
                        });
                    }
                    if let Some(index) = removed {
                        legs.remove(index);
                    }
                    if let Some(foot) = from
                        && ui.button(format!("+ Leg ending at {foot}")).clicked()
                    {
                        let name = format!("leg {}", legs.len() + 1);
                        legs.push(leg_to(rig, foot, &name));
                    }
                });
                ui.end_row();
            }
            Layer::Chain {
                from,
                follow,
                stiffness,
                drag,
                gravity,
                ..
            } => {
                ui.label(theme::muted("from"));
                joint_field(ui, "chain-from", from);
                ui.end_row();
                ui.label(theme::muted("follows the path"));
                ui.checkbox(follow, "");
                ui.end_row();
                number(ui, "stiffness", stiffness, 0.01, 0.0..=4.0);
                number(ui, "drag", drag, 0.01, 0.0..=1.0);
                number(ui, "gravity", gravity, 0.05, 0.0..=10.0);
            }
            Layer::Wave {
                from,
                amplitude,
                hz,
                lag,
                ..
            } => {
                ui.label(theme::muted("from"));
                joint_field(ui, "wave-from", from);
                ui.end_row();
                number(ui, "amplitude · rad", &mut amplitude.0, 0.01, 0.0..=1.5);
                number(ui, "hz", &mut hz.0, 0.01, 0.05..=10.0);
                number(ui, "lag", lag, 0.01, 0.0..=4.0);
            }
            Layer::Squash {
                joint,
                by_speed,
                on_step,
                ease,
                ..
            } => {
                ui.label(theme::muted("joint"));
                joint_field(ui, "squash-joint", joint);
                ui.end_row();
                number(ui, "by speed", by_speed, 0.005, 0.0..=1.0);
                number(ui, "on step", on_step, 0.005, 0.0..=1.0);
                number(ui, "ease · s", &mut ease.0, 0.01, 0.0..=2.0);
            }
        });
    let during = match layer {
        Layer::Feet { during, .. }
        | Layer::Chain { during, .. }
        | Layer::Wave { during, .. }
        | Layer::Squash { during, .. } => during,
    };
    ui.horizontal(|ui| {
        ui.label(theme::muted("during"));
        for action in CALM {
            let mut on = during.contains(&action);
            if ui.checkbox(&mut on, action.as_ref()).changed() {
                if on {
                    during.push(action);
                } else {
                    during.retain(|other| *other != action);
                }
            }
        }
    });
}

fn number(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.label(theme::muted(label));
    ui.add(
        egui::DragValue::new(value)
            .speed(speed)
            .range(range)
            .fixed_decimals(3),
    );
    ui.end_row();
}

/// The stage: a body wearing the skeleton walking a loop round its middle in the chosen action,
/// its layers running as the client runs them.
fn walk(editor: &mut Editor, ui: &mut Ui, key: &str, def: &SkeletonDef, rig: &NodeTree) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let action = editor
        .views
        .skeleton
        .view(key)
        .layers
        .action
        .unwrap_or(Action::Walk);
    let mut picked = action;
    let mut calm = editor.views.skeleton.view(key).layers.reduce_motion;
    super::wearer_picker(editor, ui, key, &content);
    ui.horizontal(|ui| {
        ui.label(theme::muted("action"));
        for other in CALM {
            if chip::mode(ui, other.as_ref(), picked == other).clicked() {
                picked = other;
            }
        }
        ui.add_space(8.0);
        if chip::toggle(ui, "Reduce motion", calm).clicked() {
            calm = !calm;
        }
    });
    let view = editor.views.skeleton.view_mut(key);
    view.layers.reduce_motion = calm;
    if picked != action {
        view.layers.action = Some(picked);
        view.scrub.restart();
    }
    let Some((model_path, model)) = super::wearer(editor, key, &content) else {
        form::hint(
            ui,
            "No model wearing the skeleton loaded, so nothing walks: the skeleton loads once every \
             action has a clip on the sets page and a model wears it.",
        );
        return;
    };
    let resolved = model.resolve(None, &Motion::Action(picked), Pose::Idle, 0);
    let length = resolved.length.millis();
    let scrub = &mut editor.views.skeleton.view_mut(key).scrub;
    scrub.playing = true;
    crate::views::cues::advance(ui, scrub, Some(length));
    let playhead = scrub.playhead;
    let crossed = scrub.crossed(resolved.cues, length);
    let stepped = crossed.iter().any(|cue| cue.step);
    let time = ui.input(|i| i.time) as f32;
    let speed = match resolved.pace {
        Some(pace) if picked.moves() => pace.0.0 * model.scale(),
        _ => 0.0,
    };
    let model_id = content
        .by_path::<ModelDef>(&model_path)
        .expect("a wearer is a model of the game");
    let placed = around(time * speed / LOOP, speed > 0.0);
    let width = ui.available_width();
    let size = Vec2::new(width, (ui.available_height() - 40.0).min(width));
    let layered = StageLayers {
        model: model_id,
        action: picked,
        stepped,
        reduce_motion: calm,
    };
    let clip = StageClip::played(model.skeleton(), resolved.played);
    stage_walking(
        editor, ui, key, size, model, clip, playhead, crossed, placed, layered,
    );
    let refusals = refusals(rig, &def.layers);
    if def.layers.is_empty() {
        form::hint(ui, "No layers to run: the body plays its clip alone.");
    } else if refusals.is_empty() {
        form::hint(
            ui,
            "The body walks a loop on flat ground with its layers running as the client runs them.",
        );
    }
}

/// The page's stage over `size` with `model` placed by `placed`, playing `clip` at `playhead` and
/// sounding the cues it `crossed`, with its skeleton's layers running as `layered` tells them.
#[allow(clippy::too_many_arguments)]
fn stage_walking(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    size: Vec2,
    model: &ActorModel,
    clip: StageClip,
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

/// Where a body stands `turned` radians round the loop, facing along it when it walks.
fn around(turned: f32, walking: bool) -> Transform {
    let center = stage::center();
    let at = if walking {
        Pos::new(
            center.x + LOOP * turned.cos(),
            center.y + LOOP * turned.sin(),
        )
    } else {
        center
    };
    let heading = if walking {
        Quat::from_rotation_y(-turned)
    } else {
        Quat::IDENTITY
    };
    Transform {
        translation: world_point(at, Meters(0.0)),
        rotation: heading,
        ..Transform::default()
    }
}
