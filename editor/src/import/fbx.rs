//! An FBX file as a glTF binary: its nodes at rest, its meshes with their skins and materials, and
//! every take it animates, in metres with Y up, as a game reads models.

use std::collections::{BTreeMap, HashMap};

use engine::core::content::gltf::{glb, push_accessor};
use gltf::json;
use gltf::json::validation::{Checked, USize64};

pub fn glb_of_fbx(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let scene = ufbx::load_memory(
        bytes,
        ufbx::LoadOpts {
            target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            target_unit_meters: 1.0,
            space_conversion: ufbx::SpaceConversion::ModifyGeometry,
            // A glTF node always takes its parent's whole transform, and a helper node standing
            // in for anything else would be a joint of no name that no rig has.
            geometry_transform_handling: ufbx::GeometryTransformHandling::ModifyGeometryNoFallback,
            inherit_mode_handling: ufbx::InheritModeHandling::CompensateNoFallback,
            generate_missing_normals: true,
            clean_skin_weights: true,
            ..Default::default()
        },
    )
    .map_err(|error| error.description.to_string())?;
    let mut out = Out::default();
    let nodes = out.nodes(&scene);
    let mut materials = HashMap::new();
    for node in scene.nodes.iter().filter(|node| !node.is_root) {
        let Some(mesh) = &node.mesh else {
            continue;
        };
        let at = nodes[&node.element.typed_id];
        if let Some(drawn) = out.mesh(mesh, &nodes, &mut materials)? {
            out.root.nodes[at.value()].mesh = Some(drawn.mesh);
            out.root.nodes[at.value()].skin = drawn.skin;
        }
    }
    let roots = scene
        .root_node
        .children
        .iter()
        .map(|child| nodes[&child.element.typed_id])
        .collect();
    let scene_index = out.root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: None,
        nodes: roots,
    });
    out.root.scene = Some(scene_index);
    for stack in &scene.anim_stacks {
        out.animation(&scene, stack, &nodes)?;
    }
    Ok(out.finish())
}

#[derive(Default)]
struct Out {
    root: json::Root,
    bin: Vec<u8>,
}

/// A node's mesh, and the skin it is drawn through when it has one.
struct Drawn {
    mesh: json::Index<json::Mesh>,
    skin: Option<json::Index<json::Skin>>,
}

/// What a corner of a triangle is made of, so corners alike share a vertex.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Corner {
    vertex: u32,
    normal: Option<u32>,
    uv: Option<u32>,
}

const WEIGHTS_PER_VERTEX: usize = 4;

/// How near a key must come to the node's rest to leave it where it rests.
const STILL: f32 = 1e-5;

/// The keys of one property of a node through a take, each `rest.len()` floats wide.
struct Track {
    path: json::animation::Property,
    kind: json::accessor::Type,
    times: Vec<f64>,
    values: Vec<f32>,
    rest: Vec<f32>,
}

impl Track {
    /// Whether the take leaves the node as it rests, as a mesh's file keeps a take of its bind. A
    /// turn and its negation turn alike.
    fn still(&self) -> bool {
        let near = |key: &[f32], sign: f32| {
            key.iter()
                .zip(&self.rest)
                .all(|(at, rest)| (sign * at - rest).abs() < STILL)
        };
        let turns = self.path == json::animation::Property::Rotation;
        self.values
            .chunks(self.rest.len())
            .all(|key| near(key, 1.0) || (turns && near(key, -1.0)))
    }
}

impl Out {
    fn nodes(&mut self, scene: &ufbx::Scene) -> HashMap<u32, json::Index<json::Node>> {
        let mut nodes = HashMap::new();
        for node in scene.nodes.iter().filter(|node| !node.is_root) {
            let transform = &node.local_transform;
            let index = self.root.push(json::Node {
                name: Some(node.element.name.to_string()),
                translation: Some(vec3(transform.translation)),
                rotation: Some(json::scene::UnitQuaternion(quat(transform.rotation))),
                scale: Some(vec3(transform.scale)),
                ..Default::default()
            });
            nodes.insert(node.element.typed_id, index);
        }
        for node in scene.nodes.iter().filter(|node| !node.is_root) {
            let children: Vec<_> = node
                .children
                .iter()
                .map(|child| nodes[&child.element.typed_id])
                .collect();
            if !children.is_empty() {
                self.root.nodes[nodes[&node.element.typed_id].value()].children = Some(children);
            }
        }
        nodes
    }

    fn mesh(
        &mut self,
        mesh: &ufbx::Mesh,
        nodes: &HashMap<u32, json::Index<json::Node>>,
        materials: &mut HashMap<u32, json::Index<json::Material>>,
    ) -> Result<Option<Drawn>, String> {
        let skin = mesh.skin_deformers.first();
        let whole: Vec<u32> = (0..mesh.faces.len() as u32).collect();
        let parts: Vec<(Option<&ufbx::Material>, &[u32])> = if mesh.material_parts.is_empty() {
            vec![(None, &whole)]
        } else {
            mesh.material_parts
                .iter()
                .map(|part| {
                    (
                        mesh.materials.get(part.index as usize).map(|m| &**m),
                        &*part.face_indices,
                    )
                })
                .collect()
        };
        let mut primitives = Vec::new();
        let mut triangle = vec![0u32; mesh.max_face_triangles * 3];
        for (material, faces) in parts {
            let mut corners: HashMap<Corner, u32> = HashMap::new();
            let mut positions: Vec<[f32; 3]> = Vec::new();
            let mut normals: Vec<[f32; 3]> = Vec::new();
            let mut uvs: Vec<[f32; 2]> = Vec::new();
            let mut joints: Vec<[u16; WEIGHTS_PER_VERTEX]> = Vec::new();
            let mut weights: Vec<[f32; WEIGHTS_PER_VERTEX]> = Vec::new();
            let mut indices: Vec<u32> = Vec::new();
            for &face in faces {
                let count = ufbx::triangulate_face(&mut triangle, mesh, mesh.faces[face as usize]);
                for &corner in &triangle[..count as usize * 3] {
                    let at = corner as usize;
                    let key = Corner {
                        vertex: mesh.vertex_indices[at],
                        normal: mesh
                            .vertex_normal
                            .exists
                            .then(|| mesh.vertex_normal.indices[at]),
                        uv: mesh.vertex_uv.exists.then(|| mesh.vertex_uv.indices[at]),
                    };
                    let vertex = *corners.entry(key).or_insert_with(|| {
                        positions.push(vec3(mesh.vertex_position[at]));
                        if mesh.vertex_normal.exists {
                            normals.push(vec3(mesh.vertex_normal[at]));
                        }
                        if mesh.vertex_uv.exists {
                            let uv = mesh.vertex_uv[at];
                            uvs.push([uv.x as f32, 1.0 - uv.y as f32]);
                        }
                        if let Some(skin) = skin {
                            let (bones, strengths) = weighed(skin, key.vertex);
                            joints.push(bones);
                            weights.push(strengths);
                        }
                        positions.len() as u32 - 1
                    });
                    indices.push(vertex);
                }
            }
            if indices.is_empty() {
                continue;
            }
            let mut attributes = BTreeMap::new();
            let (min, max) = bounds(&positions);
            attributes.insert(
                Checked::Valid(json::mesh::Semantic::Positions),
                self.floats(&positions, json::accessor::Type::Vec3, Some((&min, &max))),
            );
            if !normals.is_empty() {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Normals),
                    self.floats(&normals, json::accessor::Type::Vec3, None),
                );
            }
            if !uvs.is_empty() {
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::TexCoords(0)),
                    self.floats(&uvs, json::accessor::Type::Vec2, None),
                );
            }
            if skin.is_some() {
                let bytes: Vec<u8> = joints
                    .iter()
                    .flatten()
                    .flat_map(|joint| joint.to_le_bytes())
                    .collect();
                let accessor = self.typed(
                    &bytes,
                    joints.len(),
                    json::accessor::Type::Vec4,
                    json::accessor::ComponentType::U16,
                );
                attributes.insert(Checked::Valid(json::mesh::Semantic::Joints(0)), accessor);
                attributes.insert(
                    Checked::Valid(json::mesh::Semantic::Weights(0)),
                    self.floats(&weights, json::accessor::Type::Vec4, None),
                );
            }
            let bytes: Vec<u8> = indices
                .iter()
                .flat_map(|index| index.to_le_bytes())
                .collect();
            let indices = self.typed(
                &bytes,
                indices.len(),
                json::accessor::Type::Scalar,
                json::accessor::ComponentType::U32,
            );
            let material = material.map(|material| {
                *materials
                    .entry(material.element.element_id)
                    .or_insert_with(|| self.material(material))
            });
            primitives.push(json::mesh::Primitive {
                attributes,
                extensions: None,
                extras: Default::default(),
                indices: Some(indices),
                material,
                mode: Checked::Valid(json::mesh::Mode::Triangles),
                targets: None,
            });
        }
        if primitives.is_empty() {
            return Ok(None);
        }
        let index = self.root.push(json::Mesh {
            extensions: None,
            extras: Default::default(),
            name: Some(mesh.element.name.to_string()),
            primitives,
            weights: None,
        });
        let skin = skin.map(|skin| self.skin(skin, nodes)).transpose()?;
        Ok(Some(Drawn { mesh: index, skin }))
    }

    fn skin(
        &mut self,
        skin: &ufbx::SkinDeformer,
        nodes: &HashMap<u32, json::Index<json::Node>>,
    ) -> Result<json::Index<json::Skin>, String> {
        let joints = skin
            .clusters
            .iter()
            .map(|cluster| {
                let bone = cluster
                    .bone_node
                    .as_ref()
                    .ok_or_else(|| format!("{} binds no bone", &*cluster.element.name))?;
                Ok(nodes[&bone.element.typed_id])
            })
            .collect::<Result<_, String>>()?;
        let inverse: Vec<[f32; 16]> = skin
            .clusters
            .iter()
            .map(|cluster| matrix(&cluster.geometry_to_bone))
            .collect();
        let inverse_bind_matrices = self.floats(&inverse, json::accessor::Type::Mat4, None);
        Ok(self.root.push(json::Skin {
            extensions: None,
            extras: Default::default(),
            inverse_bind_matrices: Some(inverse_bind_matrices),
            joints,
            name: Some(skin.element.name.to_string()),
            skeleton: None,
        }))
    }

    fn material(&mut self, material: &ufbx::Material) -> json::Index<json::Material> {
        let colour = [&material.pbr.base_color, &material.fbx.diffuse_color]
            .into_iter()
            .find(|map| map.has_value)
            .map_or([1.0; 4], |map| {
                let value = map.value_vec4;
                [value.x as f32, value.y as f32, value.z as f32, 1.0]
            });
        self.root.push(json::Material {
            name: Some(material.element.name.to_string()),
            pbr_metallic_roughness: json::material::PbrMetallicRoughness {
                base_color_factor: json::material::PbrBaseColorFactor(colour),
                metallic_factor: json::material::StrengthFactor(0.0),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn animation(
        &mut self,
        scene: &ufbx::Scene,
        stack: &ufbx::AnimStack,
        nodes: &HashMap<u32, json::Index<json::Node>>,
    ) -> Result<(), String> {
        let baked = ufbx::bake_anim(
            scene,
            &stack.anim,
            ufbx::BakeOpts {
                key_reduction_enabled: true,
                ..Default::default()
            },
        )
        .map_err(|error| format!("{}: {}", &*stack.element.name, &*error.description))?;
        let start = baked.playback_time_begin;
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for keyed in baked.nodes.iter() {
            let Some(&node) = nodes.get(&keyed.typed_id) else {
                continue;
            };
            let rest = &scene.nodes[keyed.typed_id as usize].local_transform;
            let tracks = [
                Track {
                    path: json::animation::Property::Translation,
                    kind: json::accessor::Type::Vec3,
                    times: keyed.translation_keys.iter().map(|key| key.time).collect(),
                    values: keyed
                        .translation_keys
                        .iter()
                        .flat_map(|key| vec3(key.value))
                        .collect(),
                    rest: vec3(rest.translation).to_vec(),
                },
                Track {
                    path: json::animation::Property::Rotation,
                    kind: json::accessor::Type::Vec4,
                    times: keyed.rotation_keys.iter().map(|key| key.time).collect(),
                    values: keyed
                        .rotation_keys
                        .iter()
                        .flat_map(|key| quat(key.value))
                        .collect(),
                    rest: quat(rest.rotation).to_vec(),
                },
                Track {
                    path: json::animation::Property::Scale,
                    kind: json::accessor::Type::Vec3,
                    times: keyed.scale_keys.iter().map(|key| key.time).collect(),
                    values: keyed
                        .scale_keys
                        .iter()
                        .flat_map(|key| vec3(key.value))
                        .collect(),
                    rest: vec3(rest.scale).to_vec(),
                },
            ];
            for track in tracks.into_iter().filter(|track| !track.still()) {
                let bytes: Vec<u8> = track
                    .values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                let count = track.values.len() / track.rest.len();
                let output = push_accessor(
                    &mut self.root,
                    &mut self.bin,
                    &bytes,
                    count,
                    track.kind,
                    None,
                );
                let (path, times) = (track.path, track.times);
                let times: Vec<[f32; 1]> = times
                    .iter()
                    .map(|time| [(time - start).max(0.0) as f32])
                    .collect();
                let first = times[0];
                let last = times[times.len() - 1];
                let input =
                    self.floats(&times, json::accessor::Type::Scalar, Some((&first, &last)));
                channels.push(json::animation::Channel {
                    extensions: None,
                    extras: Default::default(),
                    sampler: json::Index::new(samplers.len() as u32),
                    target: json::animation::Target {
                        extensions: None,
                        extras: Default::default(),
                        node,
                        path: Checked::Valid(path),
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
        }
        if !channels.is_empty() {
            self.root.push(json::Animation {
                extensions: None,
                extras: Default::default(),
                channels,
                name: Some(stack.element.name.to_string()),
                samplers,
            });
        }
        Ok(())
    }

    fn floats<const N: usize>(
        &mut self,
        values: &[[f32; N]],
        kind: json::accessor::Type,
        bounds: Option<(&[f32; N], &[f32; N])>,
    ) -> json::Index<json::Accessor> {
        let bytes: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        push_accessor(
            &mut self.root,
            &mut self.bin,
            &bytes,
            values.len(),
            kind,
            bounds.map(|(min, max)| (&min[..], &max[..])),
        )
    }

    fn typed(
        &mut self,
        bytes: &[u8],
        count: usize,
        kind: json::accessor::Type,
        component: json::accessor::ComponentType,
    ) -> json::Index<json::Accessor> {
        let accessor = push_accessor(&mut self.root, &mut self.bin, bytes, count, kind, None);
        self.root.accessors[accessor.value()].component_type =
            Checked::Valid(json::accessor::GenericComponentType(component));
        accessor
    }

    fn finish(mut self) -> Vec<u8> {
        self.root.push(json::Buffer {
            byte_length: USize64::from(self.bin.len()),
            extensions: None,
            extras: Default::default(),
            name: None,
            uri: None,
        });
        glb(&self.root, self.bin)
    }
}

/// The bones that move `vertex` most, at most four, with their weights summing to one.
fn weighed(
    skin: &ufbx::SkinDeformer,
    vertex: u32,
) -> ([u16; WEIGHTS_PER_VERTEX], [f32; WEIGHTS_PER_VERTEX]) {
    let mut bones = [0u16; WEIGHTS_PER_VERTEX];
    let mut strengths = [0.0f32; WEIGHTS_PER_VERTEX];
    let Some(at) = skin.vertices.get(vertex as usize) else {
        return (bones, strengths);
    };
    let begin = at.weight_begin as usize;
    let mut weights: Vec<&ufbx::SkinWeight> = (&*skin.weights)
        [begin..begin + at.num_weights as usize]
        .iter()
        .collect();
    weights.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let kept = &weights[..weights.len().min(WEIGHTS_PER_VERTEX)];
    let total: f64 = kept.iter().map(|weight| weight.weight).sum();
    for (slot, weight) in kept.iter().enumerate() {
        bones[slot] = weight.cluster_index as u16;
        strengths[slot] = if total > 0.0 {
            (weight.weight / total) as f32
        } else {
            0.0
        };
    }
    (bones, strengths)
}

fn bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for position in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    (min, max)
}

fn vec3(value: ufbx::Vec3) -> [f32; 3] {
    [value.x as f32, value.y as f32, value.z as f32]
}

fn quat(value: ufbx::Quat) -> [f32; 4] {
    [
        value.x as f32,
        value.y as f32,
        value.z as f32,
        value.w as f32,
    ]
}

fn matrix(value: &ufbx::Matrix) -> [f32; 16] {
    [
        value.m00, value.m10, value.m20, 0.0, value.m01, value.m11, value.m21, 0.0, value.m02,
        value.m12, value.m22, 0.0, value.m03, value.m13, value.m23, 1.0,
    ]
    .map(|value| value as f32)
}
