//! A body's layers as its skeleton lists them, over the joints of its rig as its scene spawned
//! them, each let run while an action it lists plays.

use std::collections::HashMap;

use bevy::prelude::*;

use super::skeleton::{JointName, Layer};
use super::{Action, ActorModel};
use crate::core::content::gltf::NodeTree;
use crate::core::render::world::layers::{Chain, Feet, JointLayer, Layered, Rig, Squash, Wave};

/// The actions each of a body's layers runs during, in the order of its layers.
#[derive(Component)]
pub struct Layering {
    during: Vec<Vec<Action>>,
}

/// The layers of `model`'s skeleton over the joints of its spawned scene, found by name, if it has
/// any. A layer naming a joint the scene lacks is left out; the loader refuses such a skeleton.
pub fn layered(
    model: &ActorModel,
    joints: &HashMap<String, Entity>,
) -> Option<(Layered, Layering)> {
    let skeleton = model.skeleton();
    if skeleton.def.layers.is_empty() {
        return None;
    }
    let (rig, index) = rig(&skeleton.rig, joints)?;
    let at = |joint: &JointName| index.get(joint.as_str()).copied();
    let mut layers = Vec::new();
    let mut during = Vec::new();
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
            during.push(layer.during().to_vec());
        }
    }
    Some((Layered::new(rig, layers), Layering { during }))
}

/// Lets each layer run while the action shown is one it lists, and tells them a foot came down.
pub fn drive(layered: &mut Layered, layering: &Layering, action: Action, stepped: bool) {
    for (on, during) in layered.on.iter_mut().zip(&layering.during) {
        *on = during.contains(&action);
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
