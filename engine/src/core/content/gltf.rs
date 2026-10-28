//! What the loader learns of a glTF binary without a renderer: its bounds, how heavy it draws, its
//! tree of named nodes at rest and the clips it animates with, so a model is held to the style
//! budget, fitted to the rig it shares clips with, and its clips are known at load, on the server as
//! on the client.

use std::collections::BTreeMap;

use bevy::math::{Mat4, Quat, Vec3};
use gltf::json;
use gltf::json::validation::{Checked, USize64};

use crate::core::geometry::Degrees;
use crate::core::math::Pos;
use crate::core::time::Seconds;
use crate::core::units::Meters;

#[derive(Clone, Debug, PartialEq)]
pub struct GltfInfo {
    pub bounds: MeshBounds,
    pub triangles: usize,
    pub materials: usize,
    /// Whether any mesh is skinned to joints.
    pub skinned: bool,
    /// The nodes the skins bind their meshes to, by index in the tree.
    pub skin_joints: Vec<usize>,
    /// Each node a skin binds, by index in the tree, with the inverse of where the skin binds it:
    /// the first skin's bind where several bind the same node.
    pub binds: Vec<(usize, Mat4)>,
    pub tree: NodeTree,
    pub clips: Vec<GltfClip>,
}

/// Every node of a glTF binary by its index there, each with its name, its parent and its rest
/// transform: the joints a clip finds by the path of names from the scene's root down, as the
/// renderer finds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeTree {
    pub nodes: Vec<TreeNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub name: String,
    pub parent: Option<usize>,
    pub rest: Rest,
    /// Whether the node draws a mesh: a mesh is skin, not a joint.
    pub mesh: bool,
}

/// A node's transform relative to its parent, as it stands when nothing animates it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rest {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Rest {
    pub const IDENTITY: Rest = Rest {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// The box every mesh fits in, in the scene's metres, with every node's transform applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshBounds {
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfClip {
    pub name: String,
    pub length: Seconds,
    /// The nodes the clip moves, by their index in the tree.
    pub targets: Vec<usize>,
    /// How the clip turns each node it turns at its first moment, by the node's index in the tree.
    pub opening: Vec<(usize, Quat)>,
}

impl GltfInfo {
    pub fn clip(&self, name: &str) -> Option<&GltfClip> {
        self.clips.iter().find(|clip| clip.name == name)
    }

    pub fn clip_names(&self) -> Vec<&str> {
        self.clips.iter().map(|clip| clip.name.as_str()).collect()
    }

    /// The joints of the tree: every node that draws no mesh.
    pub fn joints(&self) -> usize {
        self.tree.joints().count()
    }
}

impl NodeTree {
    /// The nodes that draw no mesh, by index: what clips move and hands hang from.
    pub fn joints(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&index| !self.nodes[index].mesh)
    }

    /// The names from the root down to the node at `index`, its own last.
    pub fn path(&self, index: usize) -> Vec<&str> {
        let mut path = Vec::new();
        let mut at = Some(index);
        while let Some(node) = at {
            path.push(self.nodes[node].name.as_str());
            at = self.nodes[node].parent;
        }
        path.reverse();
        path
    }

    /// The node at the path of names from the root.
    pub fn at_path(&self, path: &[&str]) -> Option<usize> {
        (0..self.nodes.len()).find(|&index| self.path(index) == path)
    }

    /// The first node named `name`.
    pub fn named(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.name == name)
    }

    /// Whether the node at `descendant` lies under the node at `ancestor`, or is it.
    pub fn descends(&self, descendant: usize, ancestor: usize) -> bool {
        let mut at = Some(descendant);
        while let Some(node) = at {
            if node == ancestor {
                return true;
            }
            at = self.nodes[node].parent;
        }
        false
    }

    /// Where every node rests in the scene, by its index: its rest under each of its parents'.
    pub fn worlds(&self) -> Vec<Mat4> {
        let mut worlds: Vec<Option<Mat4>> = vec![None; self.nodes.len()];
        for index in 0..self.nodes.len() {
            self.world_into(index, &mut worlds);
        }
        worlds.into_iter().map(Option::unwrap_or_default).collect()
    }

    fn world_into(&self, index: usize, worlds: &mut [Option<Mat4>]) -> Mat4 {
        if let Some(world) = worlds[index] {
            return world;
        }
        let node = &self.nodes[index];
        let parent = node
            .parent
            .map_or(Mat4::IDENTITY, |parent| self.world_into(parent, worlds));
        let world = parent * node.rest.matrix();
        worlds[index] = Some(world);
        world
    }

    /// Every node with each of its parents before it.
    pub fn downward(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut placed = vec![false; self.nodes.len()];
        for index in 0..self.nodes.len() {
            let mut chain = Vec::new();
            let mut at = Some(index);
            while let Some(node) = at.filter(|&node| !placed[node]) {
                chain.push(node);
                at = self.nodes[node].parent;
            }
            for node in chain.into_iter().rev() {
                placed[node] = true;
                order.push(node);
            }
        }
        order
    }

    pub fn parent_name(&self, index: usize) -> &str {
        self.nodes[index]
            .parent
            .map_or("the root", |parent| self.nodes[parent].name.as_str())
    }
}

/// How far a skin may bind a joint turned from where the rig rests it and still be drawn true.
pub const REST_TURN: Degrees = Degrees(2.0);
/// How far a skin may bind a joint from where the rig rests it and still be drawn true.
pub const REST_SHIFT: Meters = Meters(0.002);

/// How a skin fits a rig: how many of the joints its meshes bind are joints of the rig resting
/// where the skin binds them, and why each other one does not fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub matched: usize,
    /// How many of the joints the skin binds the rig has.
    pub joints: usize,
    pub misfits: Vec<Misfit>,
    /// The bound joint of the rig resting furthest from where the skin binds it, and how far it turns, fitting
    /// or not.
    pub worst_rest: Option<(String, Degrees)>,
}

/// A joint a skin binds away from where the rig rests it: how far it turns and stands apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Misfit {
    pub joint: String,
    pub turn: Degrees,
    pub shift: Meters,
}

impl Fit {
    pub fn fits(&self) -> bool {
        self.misfits.is_empty()
    }
}

/// How the skins of `skin` fit `rig`: each joint they bind that the rig has, by its name, must rest
/// within `REST_TURN` and `REST_SHIFT` of where the skin binds it, the binds placed where the rig
/// rests the topmost of those joints. Exporters bind in frames of their own (an armature's, the
/// scene's), and a renderer draws a skin from its binds alone, so what fits is the binds' layout
/// against the rig's, wherever the skin's own file places its joints. A joint the rig lacks holds
/// what it binds at rest.
pub fn fit_skin(rig: &NodeTree, skin: &GltfInfo) -> Fit {
    let worlds = rig.worlds();
    let bound: Vec<(usize, usize, &Mat4)> = skin
        .binds
        .iter()
        .filter_map(|(node, inverse_bind)| {
            let name = &skin.tree.nodes[*node].name;
            let joint = rig.joints().find(|&joint| rig.nodes[joint].name == *name)?;
            Some((*node, joint, inverse_bind))
        })
        .collect();
    let order = rig.downward();
    let depth = |joint: usize| order.iter().position(|&at| at == joint);
    let space = bound
        .iter()
        .min_by_key(|(_, joint, _)| depth(*joint))
        .map_or(Mat4::IDENTITY, |(_, joint, inverse_bind)| {
            worlds[*joint] * **inverse_bind
        });
    let mut misfits = Vec::new();
    let mut matched = 0;
    let mut worst: Option<(String, Degrees)> = None;
    for (node, joint, inverse_bind) in bound {
        let name = &skin.tree.nodes[node].name;
        let (_, bound_turn, bound_at) =
            (space * inverse_bind.inverse()).to_scale_rotation_translation();
        let (_, rest_turn, rest_at) = worlds[joint].to_scale_rotation_translation();
        let turn = Degrees(rest_turn.angle_between(bound_turn).to_degrees());
        let shift = Meters(rest_at.distance(bound_at));
        if worst.as_ref().is_none_or(|(_, most)| turn > *most) {
            worst = Some((name.clone(), turn));
        }
        if turn > REST_TURN || shift > REST_SHIFT {
            misfits.push(Misfit {
                joint: name.clone(),
                turn,
                shift,
            });
        } else {
            matched += 1;
        }
    }
    Fit {
        matched,
        joints: matched + misfits.len(),
        misfits,
        worst_rest: worst,
    }
}

/// Reads `bytes` as a glTF binary, validated as the renderer would, reading of its buffers only the
/// turn each clip opens on.
pub fn inspect(bytes: &[u8]) -> Result<GltfInfo, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|error| error.to_string())?;
    let blob = gltf.blob.as_deref();
    let document = gltf.document;
    let mut bounds: Option<MeshBounds> = None;
    for scene in document.scenes() {
        for node in scene.nodes() {
            gather_bounds(&node, Mat4::IDENTITY, &mut bounds);
        }
    }
    let triangles = document
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .filter(|primitive| primitive.mode() == gltf::mesh::Mode::Triangles)
        .map(|primitive| {
            let corners = match primitive.indices() {
                Some(indices) => indices.count(),
                None => primitive
                    .get(&gltf::Semantic::Positions)
                    .map_or(0, |positions| positions.count()),
            };
            corners / 3
        })
        .sum();
    let clips = document
        .animations()
        .map(|animation| {
            let mut targets: Vec<usize> = animation
                .channels()
                .map(|channel| channel.target().node().index())
                .collect();
            targets.sort_unstable();
            targets.dedup();
            let opening = animation
                .channels()
                .filter_map(|channel| {
                    let reader = channel.reader(|buffer| match buffer.source() {
                        gltf::buffer::Source::Bin => blob,
                        gltf::buffer::Source::Uri(_) => None,
                    });
                    match reader.read_outputs()? {
                        gltf::animation::util::ReadOutputs::Rotations(turns) => Some((
                            channel.target().node().index(),
                            Quat::from_array(turns.into_f32().next()?),
                        )),
                        _ => None,
                    }
                })
                .collect();
            GltfClip {
                name: animation.name().unwrap_or_default().to_owned(),
                length: Seconds(
                    animation
                        .samplers()
                        .filter_map(|sampler| last_value(sampler.input().max()))
                        .fold(0.0, f32::max),
                ),
                targets,
                opening,
            }
        })
        .collect();
    Ok(GltfInfo {
        bounds: bounds.unwrap_or(MeshBounds {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        }),
        triangles,
        materials: document.materials().count(),
        skinned: document.skins().next().is_some(),
        skin_joints: {
            let mut joints: Vec<usize> = document
                .skins()
                .flat_map(|skin| skin.joints().map(|joint| joint.index()))
                .collect();
            joints.sort_unstable();
            joints.dedup();
            joints
        },
        binds: binds_of(&document, blob),
        tree: tree_of(&document),
        clips,
    })
}

fn binds_of(document: &gltf::Document, blob: Option<&[u8]>) -> Vec<(usize, Mat4)> {
    let mut binds: Vec<(usize, Mat4)> = Vec::new();
    for skin in document.skins() {
        let reader = skin.reader(|buffer| match buffer.source() {
            gltf::buffer::Source::Bin => blob,
            gltf::buffer::Source::Uri(_) => None,
        });
        let inverses: Vec<Mat4> = reader
            .read_inverse_bind_matrices()
            .map(|matrices| matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect())
            .unwrap_or_default();
        for (at, joint) in skin.joints().enumerate() {
            if binds.iter().any(|(node, _)| *node == joint.index()) {
                continue;
            }
            binds.push((
                joint.index(),
                inverses.get(at).copied().unwrap_or(Mat4::IDENTITY),
            ));
        }
    }
    binds
}

fn tree_of(document: &gltf::Document) -> NodeTree {
    let mut nodes: Vec<TreeNode> = document
        .nodes()
        .map(|node| {
            let (translation, rotation, scale) = node.transform().decomposed();
            TreeNode {
                name: node.name().unwrap_or_default().to_owned(),
                parent: None,
                rest: Rest {
                    translation: Vec3::from(translation),
                    rotation: Quat::from_array(rotation),
                    scale: Vec3::from(scale),
                },
                mesh: node.mesh().is_some(),
            }
        })
        .collect();
    for node in document.nodes() {
        for child in node.children() {
            nodes[child.index()].parent = Some(node.index());
        }
    }
    NodeTree { nodes }
}

/// A clip's keyframes read whole: what it does to each node it moves, by the node's index in the
/// tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipKeys {
    pub length: Seconds,
    pub tracks: Vec<NodeKeys>,
}

/// How a clip moves one node, by the node's index in the tree: whatever it leaves unkeyed rests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeKeys {
    pub node: usize,
    pub translation: Option<Keys<Vec3>>,
    pub rotation: Option<Keys<Quat>>,
    pub scale: Option<Keys<Vec3>>,
}

/// Values at moments, in seconds from the clip's start, each held until the next when `stepped`
/// and blended toward it otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct Keys<T> {
    pub times: Vec<f32>,
    pub values: Vec<T>,
    pub stepped: bool,
}

/// A value keys blend between.
pub trait Keyed: Copy {
    fn blend(self, to: Self, t: f32) -> Self;
}

impl Keyed for Vec3 {
    fn blend(self, to: Vec3, t: f32) -> Vec3 {
        self.lerp(to, t)
    }
}

impl Keyed for Quat {
    fn blend(self, to: Quat, t: f32) -> Quat {
        self.slerp(to, t)
    }
}

impl<T: Keyed> Keys<T> {
    /// The value at `time`: the first key's before it, the last key's after.
    pub fn at(&self, time: f32) -> Option<T> {
        let after = self.times.partition_point(|&key| key <= time);
        match after {
            0 => self.values.first().copied(),
            _ if after >= self.times.len() => self.values.last().copied(),
            _ => {
                let (from, to) = (after - 1, after);
                let span = self.times[to] - self.times[from];
                let t = if self.stepped || span <= 0.0 {
                    0.0
                } else {
                    (time - self.times[from]) / span
                };
                Some(self.values[from].blend(self.values[to], t))
            }
        }
    }
}

impl ClipKeys {
    /// Every moment any of the clip's keys stands at, in order, each once.
    pub fn moments(&self) -> Vec<f32> {
        let mut moments: Vec<f32> = self
            .tracks
            .iter()
            .flat_map(|track| {
                let translation = track.translation.iter().flat_map(|keys| &keys.times);
                let rotation = track.rotation.iter().flat_map(|keys| &keys.times);
                let scale = track.scale.iter().flat_map(|keys| &keys.times);
                translation.chain(rotation).chain(scale).copied()
            })
            .collect();
        moments.sort_by(f32::total_cmp);
        moments.dedup_by(|a, b| (*a - *b).abs() < 1.0e-4);
        if moments.is_empty() {
            moments.push(0.0);
        }
        moments
    }

    /// Where the clip holds every node of `tree` at `time`, relative to its parent: as keyed, or at
    /// rest where it keys nothing.
    pub fn pose(&self, tree: &NodeTree, time: f32) -> Vec<Rest> {
        let mut pose: Vec<Rest> = tree.nodes.iter().map(|node| node.rest).collect();
        for track in &self.tracks {
            let Some(rest) = pose.get_mut(track.node) else {
                continue;
            };
            if let Some(at) = track.translation.as_ref().and_then(|keys| keys.at(time)) {
                rest.translation = at;
            }
            if let Some(turn) = track.rotation.as_ref().and_then(|keys| keys.at(time)) {
                rest.rotation = turn.normalize();
            }
            if let Some(scale) = track.scale.as_ref().and_then(|keys| keys.at(time)) {
                rest.scale = scale;
            }
        }
        pose
    }
}

/// The keyframes of the animation at `index` among those of the glTF binary `bytes`. A cubic
/// spline is read through its keys, as straight blends between them.
pub fn clip_keys(bytes: &[u8], index: usize) -> Result<ClipKeys, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|error| error.to_string())?;
    let blob = gltf.blob.as_deref();
    let animation = gltf
        .document
        .animations()
        .nth(index)
        .ok_or_else(|| format!("no animation {index}"))?;
    let mut tracks: BTreeMap<usize, NodeKeys> = BTreeMap::new();
    let mut length: f32 = 0.0;
    for channel in animation.channels() {
        let reader = channel.reader(|buffer| match buffer.source() {
            gltf::buffer::Source::Bin => blob,
            gltf::buffer::Source::Uri(_) => None,
        });
        let Some(times) = reader.read_inputs() else {
            continue;
        };
        let times: Vec<f32> = times.collect();
        length = times.iter().copied().fold(length, f32::max);
        let interpolation = channel.sampler().interpolation();
        let stepped = interpolation == gltf::animation::Interpolation::Step;
        let cubic = interpolation == gltf::animation::Interpolation::CubicSpline;
        let node = channel.target().node().index();
        let track = tracks.entry(node).or_insert_with(|| NodeKeys {
            node,
            ..NodeKeys::default()
        });
        match reader.read_outputs() {
            Some(gltf::animation::util::ReadOutputs::Translations(values)) => {
                track.translation = Some(keys_of(&times, values.map(Vec3::from), cubic, stepped));
            }
            Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                let values = values.into_f32().map(Quat::from_array);
                track.rotation = Some(keys_of(&times, values, cubic, stepped));
            }
            Some(gltf::animation::util::ReadOutputs::Scales(values)) => {
                track.scale = Some(keys_of(&times, values.map(Vec3::from), cubic, stepped));
            }
            Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(_)) | None => {}
        }
    }
    Ok(ClipKeys {
        length: Seconds(length),
        tracks: tracks.into_values().collect(),
    })
}

fn keys_of<T>(
    times: &[f32],
    values: impl Iterator<Item = T>,
    cubic: bool,
    stepped: bool,
) -> Keys<T> {
    Keys {
        times: times.to_vec(),
        values: if cubic {
            values.skip(1).step_by(3).collect()
        } else {
            values.collect()
        },
        stepped,
    }
}

/// A triangle of a model as it stands in the scene, every node's transform applied: its corners,
/// and the normals its file gives them when it gives any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [Vec3; 3],
    pub normals: Option<[Vec3; 3]>,
}

/// Every triangle of `bytes`, read from its buffers.
pub fn triangles(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|error| error.to_string())?;
    let blob = gltf.blob.as_deref();
    let mut out = Vec::new();
    for scene in gltf.document.scenes() {
        for node in scene.nodes() {
            gather_triangles(&node, Mat4::IDENTITY, blob, &mut out);
        }
    }
    Ok(out)
}

/// The ground a model stands on: the convex hull, on the ground plane, of every corner within
/// `knee` of the model's lowest point, going round counter-clockwise. Empty for a model with no
/// triangles.
pub fn footing(triangles: &[Triangle], knee: Meters) -> Vec<Pos<Meters>> {
    let lowest = triangles
        .iter()
        .flat_map(|triangle| triangle.corners)
        .map(|corner| corner.y)
        .fold(f32::INFINITY, f32::min);
    let mut points: Vec<Pos<Meters>> = triangles
        .iter()
        .flat_map(|triangle| triangle.corners)
        .filter(|corner| corner.y <= lowest + knee.0)
        .map(|corner| Pos::new(corner.x, corner.z))
        .collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Pos<Meters>, a: Pos<Meters>, b: Pos<Meters>| {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    };
    let mut hull: Vec<Pos<Meters>> = Vec::with_capacity(points.len() * 2);
    for &point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
            hull.pop();
        }
        hull.push(point);
    }
    let lower = hull.len() + 1;
    for &point in points.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0
        {
            hull.pop();
        }
        hull.push(point);
    }
    hull.pop();
    hull
}

/// Whether every triangle is lit as one face: its three normals alike, or none given, which the
/// renderer takes as flat.
pub fn is_flat_shaded(triangles: &[Triangle]) -> bool {
    triangles.iter().all(|triangle| match triangle.normals {
        None => true,
        Some([a, b, c]) => a.abs_diff_eq(b, 1.0e-3) && a.abs_diff_eq(c, 1.0e-3),
    })
}

/// A glTF binary of one triangle, flat on the ground, animated by `clips` of the given lengths:
/// what stands in for a prop until a real one is dropped in, drawing nothing anyone would notice
/// and answering every question the loader asks of a glTF.
pub fn placeholder_rig(clips: &[(&str, Seconds)]) -> Vec<u8> {
    placeholder_mesh(clips, 1)
}

/// A glTF binary of `triangles` triangles stacked within a centimetre of the origin, animated by
/// `clips`: a model of any weight in triangles and nothing to look at.
pub fn placeholder_mesh(clips: &[(&str, Seconds)], triangles: usize) -> Vec<u8> {
    placeholder_body(&NodeTree::default(), clips, triangles)
}

/// How far a placeholder clip holds the joint it moves above where it rests.
pub const PLACEHOLDER_LIFT: Meters = Meters(0.25);

/// A glTF binary of the joints of `tree` at rest, `triangles` triangles stacked within a centimetre
/// of the origin beside them, skinned to the joints when there are any, and `clips` holding the
/// first joint below the top one (the top one when there is none below it, the mesh when there are
/// no joints) [`PLACEHOLDER_LIFT`] above its rest: a rig, a library of clips or a skin on it, of any
/// weight and nothing to look at.
pub fn placeholder_body(tree: &NodeTree, clips: &[(&str, Seconds)], triangles: usize) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut root = json::Root::default();
    let joints: Vec<json::Index<json::Node>> = tree
        .nodes
        .iter()
        .map(|node| {
            root.push(json::Node {
                name: Some(node.name.clone()),
                translation: Some(node.rest.translation.to_array()),
                rotation: Some(json::scene::UnitQuaternion(node.rest.rotation.to_array())),
                scale: Some(node.rest.scale.to_array()),
                ..Default::default()
            })
        })
        .collect();
    for (index, node) in tree.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            root.nodes[parent]
                .children
                .get_or_insert_with(Vec::new)
                .push(joints[index]);
        }
    }
    let mut roots: Vec<json::Index<json::Node>> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(index, _)| joints[index])
        .collect();
    let lifted = tree
        .nodes
        .iter()
        .position(|node| node.parent.is_some())
        .or((!tree.nodes.is_empty()).then_some(0));
    let mut animated = lifted.map(|index| joints[index]);
    let held = lifted.map_or(Vec3::ZERO, |index| tree.nodes[index].rest.translation)
        + Vec3::Y * PLACEHOLDER_LIFT.0;
    if triangles > 0 {
        let skinned = tree.nodes.iter().any(|node| node.parent.is_some());
        let mesh = push_mesh(&mut root, &mut bin, triangles, skinned);
        let skin = skinned.then(|| push_skin(&mut root, &mut bin, tree, &joints));
        let node = root.push(json::Node {
            mesh: Some(mesh),
            skin,
            ..Default::default()
        });
        roots.push(node);
        animated.get_or_insert(node);
    }
    let scene = root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: roots,
    });
    root.scene = Some(scene);
    for &(name, length) in clips {
        let Some(node) = animated else {
            break;
        };
        let times = [0.0f32, length.0];
        let input = push_accessor(
            &mut root,
            &mut bin,
            &times
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<u8>>(),
            2,
            json::accessor::Type::Scalar,
            Some((&[0.0], &[length.0])),
        );
        let output = push_accessor(
            &mut root,
            &mut bin,
            &[held, held]
                .iter()
                .flat_map(|at| at.to_array())
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<u8>>(),
            2,
            json::accessor::Type::Vec3,
            None,
        );
        let sampler = json::animation::Sampler {
            extensions: None,
            extras: Default::default(),
            input,
            interpolation: Checked::Valid(json::animation::Interpolation::Linear),
            output,
        };
        let channel = json::animation::Channel {
            extensions: None,
            extras: Default::default(),
            sampler: json::Index::new(0),
            target: json::animation::Target {
                extensions: None,
                extras: Default::default(),
                node,
                path: Checked::Valid(json::animation::Property::Translation),
            },
        };
        root.push(json::Animation {
            extensions: None,
            extras: Default::default(),
            channels: vec![channel],
            name: Some(name.to_owned()),
            samplers: vec![sampler],
        });
    }
    root.push(json::Buffer {
        byte_length: USize64::from(bin.len()),
        extensions: None,
        extras: Default::default(),
        name: None,
        uri: None,
    });
    glb(&root, bin)
}

/// A glTF binary of the joints of `tree` at rest and one clip, `name`, `length` long, holding each
/// joint `turns` names at its turn throughout: a held pose, such as fingers closed on a handle,
/// made without an artist.
pub fn posed_body(tree: &NodeTree, name: &str, length: Seconds, turns: &[(&str, Quat)]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut root = json::Root::default();
    let joints: Vec<json::Index<json::Node>> = tree
        .nodes
        .iter()
        .map(|node| {
            root.push(json::Node {
                name: Some(node.name.clone()),
                translation: Some(node.rest.translation.to_array()),
                rotation: Some(json::scene::UnitQuaternion(node.rest.rotation.to_array())),
                scale: Some(node.rest.scale.to_array()),
                ..Default::default()
            })
        })
        .collect();
    for (index, node) in tree.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            root.nodes[parent]
                .children
                .get_or_insert_with(Vec::new)
                .push(joints[index]);
        }
    }
    let roots = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(index, _)| joints[index])
        .collect();
    let scene = root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: roots,
    });
    root.scene = Some(scene);
    let input = push_accessor(
        &mut root,
        &mut bin,
        &[0.0f32, length.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>(),
        2,
        json::accessor::Type::Scalar,
        Some((&[0.0], &[length.0])),
    );
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    for (joint, turn) in turns {
        let Some(node) = tree.named(joint) else {
            continue;
        };
        let output = push_accessor(
            &mut root,
            &mut bin,
            &[*turn, *turn]
                .iter()
                .flat_map(|turn| turn.to_array())
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<u8>>(),
            2,
            json::accessor::Type::Vec4,
            None,
        );
        channels.push(json::animation::Channel {
            extensions: None,
            extras: Default::default(),
            sampler: json::Index::new(samplers.len() as u32),
            target: json::animation::Target {
                extensions: None,
                extras: Default::default(),
                node: joints[node],
                path: Checked::Valid(json::animation::Property::Rotation),
            },
        });
        samplers.push(json::animation::Sampler {
            extensions: None,
            extras: Default::default(),
            input,
            interpolation: Checked::Valid(json::animation::Interpolation::Linear),
            output,
        });
    }
    root.push(json::Animation {
        extensions: None,
        extras: Default::default(),
        channels,
        name: Some(name.to_owned()),
        samplers,
    });
    root.push(json::Buffer {
        byte_length: USize64::from(bin.len()),
        extensions: None,
        extras: Default::default(),
        name: None,
        uri: None,
    });
    glb(&root, bin)
}

/// The glTF binary `bytes` cut to its tree of nodes at rest and the animations named in `keep`: no
/// meshes, skins, materials or cameras, and only the data those animations read. A rig keeps none
/// of its clips, a library of clips made on the rig keeps the ones a skeleton will name.
pub fn cut(bytes: &[u8], keep: &[&str]) -> Result<Vec<u8>, String> {
    let read = gltf::binary::Glb::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut root: json::Root =
        json::deserialize::from_slice(&read.json).map_err(|error| error.to_string())?;
    if let Some(needed) = root.extensions_required.first() {
        return Err(format!("needs the glTF extension {needed}"));
    }
    let data: &[u8] = read.bin.as_deref().unwrap_or_default();
    root.animations.retain(|animation| {
        animation
            .name
            .as_deref()
            .is_some_and(|name| keep.contains(&name))
    });
    for animation in &mut root.animations {
        animation.channels.retain(|channel| {
            channel.target.path != Checked::Valid(json::animation::Property::MorphTargetWeights)
        });
    }
    let read_accessors: Vec<usize> = {
        let mut read: Vec<usize> = root
            .animations
            .iter()
            .flat_map(|animation| &animation.samplers)
            .flat_map(|sampler| [sampler.input.value(), sampler.output.value()])
            .collect();
        read.sort_unstable();
        read.dedup();
        read
    };
    let mut views: Vec<usize> = Vec::new();
    for &accessor in &read_accessors {
        let accessor = root
            .accessors
            .get(accessor)
            .ok_or("an animation reads an accessor the file lacks")?;
        if accessor.sparse.is_some() {
            return Err("an animation reads sparse data".to_owned());
        }
        let view = accessor
            .buffer_view
            .ok_or("an animation reads an accessor with no data")?;
        views.push(view.value());
    }
    views.sort_unstable();
    views.dedup();
    let mut bin: Vec<u8> = Vec::new();
    let mut kept_views = Vec::with_capacity(views.len());
    for &view in &views {
        let mut view = root
            .buffer_views
            .get(view)
            .ok_or("an accessor reads a view the file lacks")?
            .clone();
        if view.buffer.value() != 0 {
            return Err("animation data outside the file's own binary".to_owned());
        }
        let start = view.byte_offset.map_or(0, |offset| offset.0 as usize);
        let end = start + view.byte_length.0 as usize;
        let slice = data
            .get(start..end)
            .ok_or("a view runs past the file's binary")?;
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        view.byte_offset = Some(USize64::from(bin.len()));
        view.buffer = json::Index::new(0);
        bin.extend_from_slice(slice);
        kept_views.push(view);
    }
    let view_at = |old: usize| views.iter().position(|&kept| kept == old);
    let accessor_at = |old: usize| read_accessors.iter().position(|&kept| kept == old);
    let mut accessors = Vec::with_capacity(read_accessors.len());
    for &old in &read_accessors {
        let mut accessor = root.accessors[old].clone();
        accessor.buffer_view = accessor
            .buffer_view
            .and_then(|view| view_at(view.value()))
            .map(|view| json::Index::new(view as u32));
        accessors.push(accessor);
    }
    for sampler in root
        .animations
        .iter_mut()
        .flat_map(|animation| &mut animation.samplers)
    {
        for at in [&mut sampler.input, &mut sampler.output] {
            let kept = accessor_at(at.value()).expect("every read accessor is kept");
            *at = json::Index::new(kept as u32);
        }
    }
    for node in &mut root.nodes {
        node.mesh = None;
        node.skin = None;
        node.camera = None;
        node.weights = None;
    }
    root.accessors = accessors;
    root.buffer_views = kept_views;
    root.meshes.clear();
    root.skins.clear();
    root.materials.clear();
    root.textures.clear();
    root.images.clear();
    root.samplers.clear();
    root.cameras.clear();
    root.extensions_used.clear();
    while bin.is_empty() || !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    root.buffers = vec![json::Buffer {
        byte_length: USize64::from(bin.len()),
        extensions: None,
        extras: Default::default(),
        name: None,
        uri: None,
    }];
    Ok(glb(&root, bin))
}

/// A skin binding `joints`, the nodes of `tree` in order, at their rest.
/// A skin binding every node of `tree` that hangs under another, as an exporter binds the joints
/// under an armature: each at its rest, in the frame of the top it hangs under.
fn push_skin(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    tree: &NodeTree,
    joints: &[json::Index<json::Node>],
) -> json::Index<json::Skin> {
    let mut placed: Vec<Mat4> = Vec::with_capacity(tree.nodes.len());
    for node in &tree.nodes {
        placed.push(match node.parent {
            None => Mat4::IDENTITY,
            Some(parent) => placed[parent] * node.rest.matrix(),
        });
    }
    let bound: Vec<usize> = (0..tree.nodes.len())
        .filter(|&node| tree.nodes[node].parent.is_some())
        .collect();
    let inverse: Vec<u8> = bound
        .iter()
        .flat_map(|&node| placed[node].inverse().to_cols_array())
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let inverse_bind_matrices = push_accessor(
        root,
        bin,
        &inverse,
        bound.len(),
        json::accessor::Type::Mat4,
        None,
    );
    root.push(json::Skin {
        extensions: None,
        extras: Default::default(),
        inverse_bind_matrices: Some(inverse_bind_matrices),
        joints: bound.iter().map(|&node| joints[node]).collect(),
        name: None,
        skeleton: None,
    })
}

fn push_mesh(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    triangles: usize,
    skinned: bool,
) -> json::Index<json::Mesh> {
    let corners: [[f32; 3]; 3] = [[-0.01, 0.0, -0.01], [0.01, 0.0, -0.01], [0.0, 0.0, 0.01]];
    let positions: Vec<u8> = (0..triangles)
        .flat_map(|_| corners.iter().flatten())
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let position_accessor = push_accessor(
        root,
        bin,
        &positions,
        triangles * 3,
        json::accessor::Type::Vec3,
        Some((&[-0.01, 0.0, -0.01], &[0.01, 0.0, 0.01])),
    );
    let mut attributes = BTreeMap::from([(
        Checked::Valid(json::mesh::Semantic::Positions),
        position_accessor,
    )]);
    if skinned {
        let corners = triangles * 3;
        let joints = push_accessor(
            root,
            bin,
            &vec![0u8; corners * 4 * 2],
            corners,
            json::accessor::Type::Vec4,
            None,
        );
        root.accessors[joints.value()].component_type = Checked::Valid(
            json::accessor::GenericComponentType(json::accessor::ComponentType::U16),
        );
        let weights: Vec<u8> = (0..corners)
            .flat_map(|_| [1.0f32, 0.0, 0.0, 0.0])
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let weights = push_accessor(
            root,
            bin,
            &weights,
            corners,
            json::accessor::Type::Vec4,
            None,
        );
        attributes.insert(Checked::Valid(json::mesh::Semantic::Joints(0)), joints);
        attributes.insert(Checked::Valid(json::mesh::Semantic::Weights(0)), weights);
    }
    root.push(json::Mesh {
        extensions: None,
        extras: Default::default(),
        name: None,
        primitives: vec![json::mesh::Primitive {
            attributes,
            extensions: None,
            extras: Default::default(),
            indices: None,
            material: None,
            mode: Checked::Valid(json::mesh::Mode::Triangles),
            targets: None,
        }],
        weights: None,
    })
}

fn gather_triangles(node: &gltf::Node, parent: Mat4, blob: Option<&[u8]>, out: &mut Vec<Triangle>) {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<Vec3> = positions
                .map(|position| world.transform_point3(Vec3::from(position)))
                .collect();
            let normals: Option<Vec<Vec3>> = reader.read_normals().map(|normals| {
                normals
                    .map(|normal| {
                        world
                            .transform_vector3(Vec3::from(normal))
                            .normalize_or_zero()
                    })
                    .collect()
            });
            let indices: Vec<usize> = match reader.read_indices() {
                Some(indices) => indices.into_u32().map(|index| index as usize).collect(),
                None => (0..positions.len()).collect(),
            };
            for face in indices.as_chunks::<3>().0 {
                let corner = |i: usize| positions.get(face[i]).copied();
                let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
                    continue;
                };
                let normals = normals.as_ref().and_then(|normals| {
                    Some([
                        *normals.get(face[0])?,
                        *normals.get(face[1])?,
                        *normals.get(face[2])?,
                    ])
                });
                out.push(Triangle {
                    corners: [a, b, c],
                    normals,
                });
            }
        }
    }
    for child in node.children() {
        gather_triangles(&child, world, blob, out);
    }
}

fn gather_bounds(node: &gltf::Node, parent: Mat4, bounds: &mut Option<MeshBounds>) {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let Some(positions) = primitive.get(&gltf::Semantic::Positions) else {
                continue;
            };
            let (Some(min), Some(max)) = (vec3_of(positions.min()), vec3_of(positions.max()))
            else {
                continue;
            };
            for corner in 0..8 {
                let local = Vec3::new(
                    if corner & 1 == 0 { min.x } else { max.x },
                    if corner & 2 == 0 { min.y } else { max.y },
                    if corner & 4 == 0 { min.z } else { max.z },
                );
                let placed = world.transform_point3(local);
                *bounds = Some(match *bounds {
                    None => MeshBounds {
                        min: placed,
                        max: placed,
                    },
                    Some(known) => MeshBounds {
                        min: known.min.min(placed),
                        max: known.max.max(placed),
                    },
                });
            }
        }
    }
    for child in node.children() {
        gather_bounds(&child, world, bounds);
    }
}

fn vec3_of(value: Option<serde_json::Value>) -> Option<Vec3> {
    let values = value?;
    let values = values.as_array()?;
    let mut out = [0.0; 3];
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value.as_f64()? as f32;
    }
    Some(Vec3::from(out))
}

fn last_value(value: Option<serde_json::Value>) -> Option<f32> {
    value?
        .as_array()?
        .last()
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32)
}

/// An accessor of `count` floats of `kind`, `bytes` appended to `bin` as its view.
pub fn push_accessor(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    bytes: &[u8],
    count: usize,
    kind: json::accessor::Type,
    bounds: Option<(&[f32], &[f32])>,
) -> json::Index<json::Accessor> {
    let offset = bin.len();
    bin.extend_from_slice(bytes);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let view = root.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_length: USize64::from(bytes.len()),
        byte_offset: Some(USize64::from(offset)),
        byte_stride: None,
        extensions: None,
        extras: Default::default(),
        name: None,
        target: None,
    });
    let floats = |values: &[f32]| {
        serde_json::Value::Array(
            values
                .iter()
                .map(|value| serde_json::Value::from(f64::from(*value)))
                .collect(),
        )
    };
    root.push(json::Accessor {
        buffer_view: Some(view),
        byte_offset: Some(USize64(0)),
        count: USize64::from(count),
        component_type: Checked::Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        extensions: None,
        extras: Default::default(),
        type_: Checked::Valid(kind),
        min: bounds.map(|(min, _)| floats(min)),
        max: bounds.map(|(_, max)| floats(max)),
        name: None,
        normalized: false,
        sparse: None,
    })
}

/// The glTF binary of `root` over the buffer `bin`.
pub fn glb(root: &json::Root, bin: Vec<u8>) -> Vec<u8> {
    let mut document = root.to_string().expect("a glTF document").into_bytes();
    while !document.len().is_multiple_of(4) {
        document.push(b' ');
    }
    let total = 12 + 8 + document.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(document.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&document);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}
