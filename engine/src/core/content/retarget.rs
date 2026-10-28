//! Clips played on a rig other than the one they were made on. A clip made on a rig whose joints
//! are ours by name plays as it is; one made on a source rig plays through pairs of his joints and
//! ours: each paired joint of ours turns from its rest as his turns from his, measured in the frame
//! of the joint above his topmost paired one, so whatever moves the whole rig beneath that joint
//! (root motion, a rig made lying down) is left out. Both rigs must rest in the same pose.

use bevy::math::{Mat4, Quat, Vec3};

use super::gltf::{ClipKeys, Keys, NodeKeys, NodeTree, Rest};

/// `keys`, made on the rig of the file whose nodes are `file`, as `ours` plays it: each joint of
/// ours the clip keys, by name, under a parent of the same name takes the clip's keys, and every
/// other joint rests.
pub fn own(ours: &NodeTree, file: &NodeTree, keys: &ClipKeys) -> ClipKeys {
    let tracks = ours
        .joints()
        .map(|joint| {
            let node = &ours.nodes[joint];
            let keyed = keys.tracks.iter().find(|track| {
                file.nodes[track.node].name == node.name
                    && file.parent_name(track.node) == ours.parent_name(joint)
            });
            match keyed {
                Some(track) => NodeKeys {
                    node: joint,
                    translation: track
                        .translation
                        .clone()
                        .or_else(|| still(node.rest.translation)),
                    rotation: track.rotation.clone().or_else(|| still(node.rest.rotation)),
                    scale: track.scale.clone().or_else(|| still(node.rest.scale)),
                },
                None => resting(joint, &node.rest),
            }
        })
        .collect();
    ClipKeys {
        length: keys.length,
        tracks,
    }
}

/// `keys`, made on `his` rig, as `ours` plays it through `pairs`, each a joint of ours and the joint
/// of his it copies.
pub fn retarget(
    ours: &NodeTree,
    his: &NodeTree,
    pairs: &[(usize, usize)],
    keys: &ClipKeys,
) -> ClipKeys {
    let posing = Posing::new(ours, his, pairs);
    let mut tracks: Vec<NodeKeys> = ours
        .joints()
        .map(|joint| NodeKeys {
            node: joint,
            translation: Some(empty()),
            rotation: Some(empty()),
            scale: still(ours.nodes[joint].rest.scale),
        })
        .collect();
    for moment in keys.moments() {
        let pose = posing.pose(&keys.pose(his, moment));
        for track in &mut tracks {
            let rest = &pose[track.node];
            if let Some(keys) = track.translation.as_mut() {
                keys.times.push(moment);
                keys.values.push(rest.translation);
            }
            if let Some(keys) = track.rotation.as_mut() {
                keys.times.push(moment);
                keys.values.push(rest.rotation);
            }
        }
    }
    ClipKeys {
        length: keys.length,
        tracks,
    }
}

/// How each node of `ours` stands, relative to its parent, while his rig stands as `his_pose` has
/// each of his nodes, relative to its parent.
pub fn retargeted_pose(
    ours: &NodeTree,
    his: &NodeTree,
    pairs: &[(usize, usize)],
    his_pose: &[Rest],
) -> Vec<Rest> {
    Posing::new(ours, his, pairs).pose(his_pose)
}

/// What every moment of a retarget shares: both rigs at rest in the scene, ours ordered downward,
/// and the joints the rule measures by.
struct Posing<'a> {
    ours: &'a NodeTree,
    his: &'a NodeTree,
    order: Vec<usize>,
    our_rest: Vec<Mat4>,
    his_rest: Vec<Mat4>,
    paired: Vec<Option<usize>>,
    /// Our topmost paired joint, and his joint it copies.
    top: Option<(usize, usize)>,
}

const UP: usize = 1;

impl<'a> Posing<'a> {
    fn new(ours: &'a NodeTree, his: &'a NodeTree, pairs: &[(usize, usize)]) -> Posing<'a> {
        let order = ours.downward();
        let mut paired = vec![None; ours.nodes.len()];
        for &(our, their) in pairs {
            paired[our] = Some(their);
        }
        let top = order
            .iter()
            .find_map(|&joint| Some((joint, paired[joint]?)));
        Posing {
            ours,
            his,
            our_rest: ours.worlds(),
            his_rest: his.worlds(),
            order,
            paired,
            top,
        }
    }

    fn pose(&self, his_pose: &[Rest]) -> Vec<Rest> {
        let ours = self.ours;
        let Some((top_ours, top_his)) = self.top else {
            return ours.nodes.iter().map(|node| node.rest).collect();
        };
        let his_now = worlds_of(self.his, his_pose);
        let frame = self.his.nodes[top_his].parent;
        let frame_rest = frame.map_or(Mat4::IDENTITY, |frame| self.his_rest[frame]);
        let frame_now = frame.map_or(Mat4::IDENTITY, |frame| his_now[frame]);
        let frame_turn = turn_of(frame_rest);
        let into_now = frame_now.inverse();
        let into_rest = frame_rest.inverse();
        let his_at = |joint: usize| (frame_rest * into_now * his_now[joint]).w_axis.truncate();
        let his_rest_at = |joint: usize| self.his_rest[joint].w_axis.truncate();
        let our_turn = |joint: usize| turn_of(self.our_rest[joint]);
        let our_at = |joint: usize| self.our_rest[joint].w_axis.truncate();
        let scale = match his_rest_at(top_his)[UP] {
            height if height.abs() > 1.0e-6 => our_at(top_ours)[UP] / height,
            _ => 1.0,
        };
        let mut turn = vec![Quat::IDENTITY; ours.nodes.len()];
        let mut at = vec![Vec3::ZERO; ours.nodes.len()];
        for &joint in &self.order {
            let parent = ours.nodes[joint].parent;
            turn[joint] = match (self.paired[joint], parent) {
                (Some(his), _) => {
                    let now = turn_of(into_now * his_now[his]);
                    let rest = turn_of(into_rest * self.his_rest[his]);
                    (frame_turn * now * rest.inverse() * frame_turn.inverse() * our_turn(joint))
                        .normalize()
                }
                (None, Some(parent)) => {
                    (turn[parent] * our_turn(parent).inverse() * our_turn(joint)).normalize()
                }
                (None, None) => our_turn(joint),
            };
            at[joint] = match parent {
                None => our_at(joint),
                Some(_) if joint == top_ours => {
                    let mut travel = (his_at(top_his) - his_rest_at(top_his)) * scale;
                    travel[UP] = 0.0;
                    our_at(joint) + travel
                }
                Some(parent) => {
                    at[parent]
                        + turn[parent]
                            * our_turn(parent).inverse()
                            * (our_at(joint) - our_at(parent))
                }
            };
        }
        let paired: Vec<(usize, usize)> = self
            .paired
            .iter()
            .enumerate()
            .filter_map(|(joint, his)| Some((joint, (*his)?)))
            .collect();
        let lowest = |height: &dyn Fn(usize, usize) -> f32| {
            paired
                .iter()
                .map(|&(joint, his)| height(joint, his))
                .fold(f32::INFINITY, f32::min)
        };
        let ours_low = lowest(&|joint, _| at[joint][UP]);
        let ours_low_rest = lowest(&|joint, _| our_at(joint)[UP]);
        let his_low = lowest(&|_, his| his_at(his)[UP]);
        let his_low_rest = lowest(&|_, his| his_rest_at(his)[UP]);
        let lift = (his_low - his_low_rest) * scale - (ours_low - ours_low_rest);
        for (joint, node) in ours.nodes.iter().enumerate() {
            if node.parent.is_some() || joint == top_ours {
                at[joint][UP] += lift;
            }
        }
        ours.nodes
            .iter()
            .enumerate()
            .map(|(joint, node)| match node.parent {
                None => Rest {
                    translation: at[joint],
                    rotation: turn[joint],
                    scale: node.rest.scale,
                },
                Some(parent) => {
                    let back = turn[parent].inverse();
                    Rest {
                        translation: back * (at[joint] - at[parent])
                            / scale_of(self.our_rest[parent]),
                        rotation: (back * turn[joint]).normalize(),
                        scale: node.rest.scale,
                    }
                }
            })
            .collect()
    }
}

fn worlds_of(tree: &NodeTree, pose: &[Rest]) -> Vec<Mat4> {
    let mut worlds = vec![Mat4::IDENTITY; tree.nodes.len()];
    for node in tree.downward() {
        let parent = tree.nodes[node]
            .parent
            .map_or(Mat4::IDENTITY, |parent| worlds[parent]);
        worlds[node] = parent * pose[node].matrix();
    }
    worlds
}

fn turn_of(matrix: Mat4) -> Quat {
    matrix.to_scale_rotation_translation().1.normalize()
}

fn scale_of(matrix: Mat4) -> Vec3 {
    let scale = matrix.to_scale_rotation_translation().0;
    Vec3::select(scale.abs().cmpgt(Vec3::splat(1.0e-6)), scale, Vec3::ONE)
}

fn resting(joint: usize, rest: &Rest) -> NodeKeys {
    NodeKeys {
        node: joint,
        translation: still(rest.translation),
        rotation: still(rest.rotation),
        scale: still(rest.scale),
    }
}

fn still<T>(value: T) -> Option<Keys<T>> {
    Some(Keys {
        times: vec![0.0],
        values: vec![value],
        stepped: true,
    })
}

fn empty<T>() -> Keys<T> {
    Keys {
        times: Vec::new(),
        values: Vec::new(),
        stepped: false,
    }
}
