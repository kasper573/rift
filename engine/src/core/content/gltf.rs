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

    pub fn parent_name(&self, index: usize) -> &str {
        self.nodes[index]
            .parent
            .map_or("the root", |parent| self.nodes[parent].name.as_str())
    }
}

/// How far a joint's rest may turn from the rig's and still play the rig's clips true.
pub const REST_TURN: Degrees = Degrees(2.0);
/// How far a joint's rest may stand from the rig's and still play the rig's clips true.
pub const REST_SHIFT: Meters = Meters(0.002);

/// How a candidate's tree fits a rig's: how many of the rig's joints it has at their path, resting
/// as the rig rests them, and why each other one does not fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub matched: usize,
    pub joints: usize,
    pub misfits: Vec<Misfit>,
    /// The matched joint resting furthest from the rig's, and how far it turns, fitting or not.
    pub worst_rest: Option<(String, Degrees)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Misfit {
    Missing {
        joint: String,
    },
    Misparented {
        joint: String,
        parent: String,
        expected: String,
    },
    RestsOff {
        joint: String,
        turn: Degrees,
        shift: Meters,
    },
}

impl Fit {
    pub fn fits(&self) -> bool {
        self.misfits.is_empty()
    }
}

impl Misfit {
    pub fn joint(&self) -> &str {
        match self {
            Misfit::Missing { joint }
            | Misfit::Misparented { joint, .. }
            | Misfit::RestsOff { joint, .. } => joint,
        }
    }
}

/// How `candidate` fits `rig`: every joint of the rig must be in the candidate at the same path of
/// names from the root, which is how a clip finds the joint it moves, resting within `REST_TURN` and
/// `REST_SHIFT` of the rig's. The candidate may hold more; what is beneath a joint that does not fit
/// is not reported again.
pub fn fit(rig: &NodeTree, candidate: &NodeTree) -> Fit {
    let mut misfits = Vec::new();
    let mut matched = 0;
    let mut worst: Option<(String, Degrees)> = None;
    let mut unfit: Vec<usize> = Vec::new();
    let joints: Vec<usize> = rig.joints().collect();
    for &joint in &joints {
        let node = &rig.nodes[joint];
        if node.parent.is_some_and(|parent| unfit.contains(&parent)) {
            unfit.push(joint);
            continue;
        }
        let Some(found) = candidate.at_path(&rig.path(joint)) else {
            unfit.push(joint);
            misfits.push(match candidate.named(&node.name) {
                Some(elsewhere) => Misfit::Misparented {
                    joint: node.name.clone(),
                    parent: candidate.parent_name(elsewhere).to_owned(),
                    expected: rig.parent_name(joint).to_owned(),
                },
                None => Misfit::Missing {
                    joint: node.name.clone(),
                },
            });
            continue;
        };
        let rest = candidate.nodes[found].rest;
        let turn = Degrees(node.rest.rotation.angle_between(rest.rotation).to_degrees());
        let shift = Meters(node.rest.translation.distance(rest.translation));
        if worst.as_ref().is_none_or(|(_, most)| turn > *most) {
            worst = Some((node.name.clone(), turn));
        }
        if turn > REST_TURN || shift > REST_SHIFT {
            misfits.push(Misfit::RestsOff {
                joint: node.name.clone(),
                turn,
                shift,
            });
        } else {
            matched += 1;
        }
    }
    Fit {
        matched,
        joints: joints.len(),
        misfits,
        worst_rest: worst,
    }
}

/// Reads `bytes` as a glTF binary, validated as the renderer would, without touching its buffers.
pub fn inspect(bytes: &[u8]) -> Result<GltfInfo, String> {
    let document = gltf::Gltf::from_slice(bytes)
        .map_err(|error| error.to_string())?
        .document;
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
            GltfClip {
                name: animation.name().unwrap_or_default().to_owned(),
                length: Seconds(
                    animation
                        .samplers()
                        .filter_map(|sampler| last_value(sampler.input().max()))
                        .fold(0.0, f32::max),
                ),
                targets,
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
        tree: tree_of(&document),
        clips,
    })
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
/// of the origin beside them, and `clips` holding the first joint below the top one (the top one
/// when there is none below it, the mesh when there are no joints) [`PLACEHOLDER_LIFT`] above its
/// rest: a rig, a library of clips or a skin on it, of any weight and nothing to look at.
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
        let mesh = push_mesh(&mut root, &mut bin, triangles);
        let node = root.push(json::Node {
            mesh: Some(mesh),
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

fn push_mesh(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    triangles: usize,
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
    root.push(json::Mesh {
        extensions: None,
        extras: Default::default(),
        name: None,
        primitives: vec![json::mesh::Primitive {
            attributes: BTreeMap::from([(
                Checked::Valid(json::mesh::Semantic::Positions),
                position_accessor,
            )]),
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

fn push_accessor(
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

fn glb(root: &json::Root, bin: Vec<u8>) -> Vec<u8> {
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
