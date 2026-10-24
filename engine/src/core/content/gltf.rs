//! What the loader learns of a glTF binary without a renderer: its bounds, how heavy it draws, and
//! the clips it animates with, so a model is held to the style budget and its clips are known at
//! load, on the server as on the client.

use std::collections::BTreeMap;

use bevy::math::{Mat4, Vec3};
use gltf::json;
use gltf::json::validation::{Checked, USize64};

use crate::core::time::Seconds;

#[derive(Clone, Debug, PartialEq)]
pub struct GltfInfo {
    pub bounds: MeshBounds,
    pub triangles: usize,
    pub materials: usize,
    /// The most joints any one skin has.
    pub joints: usize,
    pub clips: Vec<GltfClip>,
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
}

impl GltfInfo {
    pub fn clip(&self, name: &str) -> Option<&GltfClip> {
        self.clips.iter().find(|clip| clip.name == name)
    }

    pub fn clip_names(&self) -> Vec<&str> {
        self.clips.iter().map(|clip| clip.name.as_str()).collect()
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
    let joints = document
        .skins()
        .map(|skin| skin.joints().count())
        .max()
        .unwrap_or(0);
    let clips = document
        .animations()
        .map(|animation| GltfClip {
            name: animation.name().unwrap_or_default().to_owned(),
            length: Seconds(
                animation
                    .samplers()
                    .filter_map(|sampler| last_value(sampler.input().max()))
                    .fold(0.0, f32::max),
            ),
        })
        .collect();
    Ok(GltfInfo {
        bounds: bounds.unwrap_or(MeshBounds {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        }),
        triangles,
        materials: document.materials().count(),
        joints,
        clips,
    })
}

/// A glTF binary of one triangle, flat on the ground, animated by `clips` of the given lengths:
/// what stands in for a model until a real one is dropped in, drawing nothing anyone would notice
/// and answering every question the loader asks of a rig.
pub fn placeholder_rig(clips: &[(&str, Seconds)]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut root = json::Root::default();
    let positions: [[f32; 3]; 3] = [[-0.01, 0.0, -0.01], [0.01, 0.0, -0.01], [0.0, 0.0, 0.01]];
    let position_accessor = push_accessor(
        &mut root,
        &mut bin,
        &positions
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>(),
        3,
        json::accessor::Type::Vec3,
        Some((&[-0.01, 0.0, -0.01], &[0.01, 0.0, 0.01])),
    );
    let mesh = root.push(json::Mesh {
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
    });
    let node = root.push(json::Node {
        mesh: Some(mesh),
        ..Default::default()
    });
    let scene = root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: vec![node],
    });
    root.scene = Some(scene);
    for &(name, length) in clips {
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
            &[0u8; 24],
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
