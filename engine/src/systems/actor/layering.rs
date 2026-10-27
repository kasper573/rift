//! A body's layers as its skeleton lists them, over the joints of its rig as its scene spawned
//! them, each let run while an action it lists plays; and each hand's clasp, held while the hand
//! holds an item.

use std::collections::HashMap;

use bevy::prelude::*;

use super::skeleton::{JointName, Layer};
use super::stance::HandIndex;
use super::{Action, ActorModel};
use crate::core::content::gltf::NodeTree;
use crate::core::render::world::layers::{
    Chain, Feet, HeldPose, JointLayer, Layered, Rig, Squash, Wave,
};

/// How long a hand takes to close on an item it takes up.
const CLASP_EASE: f32 = 0.15;

/// When each of a body's layers runs, in the order of its layers.
#[derive(Component)]
pub struct Layering {
    runs: Vec<Runs>,
}

enum Runs {
    During(Vec<Action>),
    Holding(HandIndex),
}

/// The layers of `model`'s skeleton over the joints of its spawned scene, found by name, and the
/// clasps of its hands, if it has any. A layer naming a joint the scene lacks is left out; the
/// loader refuses such a skeleton.
pub fn layered(
    model: &ActorModel,
    joints: &HashMap<String, Entity>,
) -> Option<(Layered, Layering)> {
    let skeleton = model.skeleton();
    if skeleton.def.layers.is_empty() && skeleton.clasps.iter().all(Vec::is_empty) {
        return None;
    }
    let (rig, index) = rig(&skeleton.rig, joints)?;
    let at = |joint: &JointName| index.get(joint.as_str()).copied();
    let mut layers = Vec::new();
    let mut runs = Vec::new();
    for layer in &skeleton.def.layers {
        let built = match layer {
            Layer::Feet {
                legs,
                gait,
                stride,
                lift,
                ..
            } => {
                let legs: Option<Vec<Vec<u16>>> = legs
                    .iter()
                    .map(|leg| leg.joints.iter().map(at).collect())
                    .collect();
                legs.map(|legs| JointLayer::Feet(Feet::new(&rig, &legs, *gait, stride.0, lift.0)))
            }
            Layer::Chain {
                from,
                follow,
                stiffness,
                drag,
                gravity,
                ..
            } => at(from)
                .and_then(|from| Chain::new(&rig, from, *follow, *stiffness, *drag, *gravity))
                .map(JointLayer::Chain),
            Layer::Wave {
                from,
                amplitude,
                hz,
                lag,
                ..
            } => at(from)
                .map(|from| JointLayer::Wave(Wave::new(&rig, from, amplitude.0, hz.0, *lag))),
            Layer::Squash {
                joint,
                by_speed,
                on_step,
                ease,
                ..
            } => at(joint)
                .map(|joint| JointLayer::Squash(Squash::new(joint, *by_speed, *on_step, ease.0))),
        };
        if let Some(built) = built {
            layers.push(built);
            runs.push(Runs::During(layer.during().to_vec()));
        }
    }
    for (hand, clasp) in skeleton.clasps.iter().enumerate() {
        let held: Option<Vec<(u16, glam::Quat)>> = clasp
            .iter()
            .map(|(joint, turn)| {
                let at = index
                    .get(skeleton.rig.nodes[*joint].name.as_str())
                    .copied()?;
                Some((at, glam::Quat::from_array(turn.to_array())))
            })
            .collect();
        if let Some(held) = held.filter(|held| !held.is_empty()) {
            layers.push(JointLayer::HeldPose(HeldPose::new(held, CLASP_EASE)));
            runs.push(Runs::Holding(HandIndex(hand as u8)));
        }
    }
    Some((Layered::new(rig, layers), Layering { runs }))
}

/// Lets each layer run while the action shown is one it lists, or the hand it clasps with holds an
/// item, and tells them a foot came down.
pub fn drive(
    layered: &mut Layered,
    layering: &Layering,
    action: Action,
    holding: &[HandIndex],
    stepped: bool,
) {
    for (on, runs) in layered.on.iter_mut().zip(&layering.runs) {
        *on = match runs {
            Runs::During(during) => during.contains(&action),
            Runs::Holding(hand) => holding.contains(hand),
        };
    }
    layered.stepped |= stepped;
}

/// The joints of `tree` as the scene spawned them, parents before children, and each joint's index
/// among them by name.
fn rig<'a>(
    tree: &'a NodeTree,
    joints: &HashMap<String, Entity>,
) -> Option<(Rig, HashMap<&'a str, u16>)> {
    let mut order: Vec<usize> = tree.joints().collect();
    order.sort_by_key(|&node| tree.path(node).len());
    let index: HashMap<usize, u16> = order
        .iter()
        .enumerate()
        .map(|(at, &node)| (node, at as u16))
        .collect();
    let rig = Rig {
        joints: order
            .iter()
            .map(|&node| joints.get(&tree.nodes[node].name).copied())
            .collect::<Option<Vec<Entity>>>()?,
        parents: order
            .iter()
            .map(|&node| {
                tree.nodes[node]
                    .parent
                    .and_then(|parent| index.get(&parent).copied())
            })
            .collect(),
        rest: order
            .iter()
            .map(|&node| {
                let rest = tree.nodes[node].rest;
                Transform {
                    translation: rest.translation,
                    rotation: rest.rotation,
                    scale: rest.scale,
                }
            })
            .collect(),
    };
    let names = order
        .iter()
        .enumerate()
        .map(|(at, &node)| (tree.nodes[node].name.as_str(), at as u16))
        .collect();
    Some((rig, names))
}
