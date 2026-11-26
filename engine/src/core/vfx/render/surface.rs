//! Points spread evenly over the surface of what a vfx plays on, by area, so part emitters cover a
//! blade as well as its hilt and a body as well as its hands. A skinned mesh's points ride its
//! skin as it animates.

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;

/// Where on a mesh a point lies: a triangle's corners and its weights over them, the same in any
/// pose.
#[derive(Clone, Copy)]
pub struct Sample {
    corners: [usize; 3],
    weights: [f32; 3],
}

/// `count` samples spread over a mesh by area, with the mesh's area in its own space.
pub fn spread(mesh: &Mesh, count: usize) -> Option<(f32, Vec<Sample>)> {
    let positions = positions(mesh)?;
    let triangles: Vec<[usize; 3]> = match mesh.indices() {
        Some(indices) => {
            let flat: Vec<usize> = match indices {
                Indices::U16(i) => i.iter().map(|v| *v as usize).collect(),
                Indices::U32(i) => i.iter().map(|v| *v as usize).collect(),
            };
            flat.as_chunks::<3>().0.to_vec()
        }
        None => (0..positions.len() / 3)
            .map(|t| [t * 3, t * 3 + 1, t * 3 + 2])
            .collect(),
    };
    let mut total = 0.0;
    let reach: Vec<f32> = triangles
        .iter()
        .map(|[a, b, c]| {
            let (a, b, c) = (
                Vec3::from(positions[*a]),
                Vec3::from(positions[*b]),
                Vec3::from(positions[*c]),
            );
            total += (b - a).cross(c - a).length() * 0.5;
            total
        })
        .collect();
    if total <= 0.0 {
        return None;
    }
    let samples = (0..count)
        .map(|k| {
            let at = (k as f32 + hash(k, 0)) / count as f32 * total;
            let triangle = reach.partition_point(|r| *r < at).min(triangles.len() - 1);
            let (r1, r2) = (hash(k, 1).sqrt(), hash(k, 2));
            Sample {
                corners: triangles[triangle],
                weights: [1.0 - r1, r1 * (1.0 - r2), r1 * r2],
            }
        })
        .collect();
    Some((total, samples))
}

/// Where `sample` lies in the world on a mesh drawn at `global`, posed by `skin` when it has one,
/// and the way its triangle faces there.
pub fn locate(
    mesh: &Mesh,
    sample: &Sample,
    global: &GlobalTransform,
    skin: Option<(&SkinnedMesh, &SkinnedMeshInverseBindposes)>,
    joint: &dyn Fn(Entity) -> Option<Mat4>,
) -> Option<(Vec3, Vec3)> {
    let positions = positions(mesh)?;
    let mut corners = [Vec3::ZERO; 3];
    for (placed, corner) in corners.iter_mut().zip(sample.corners) {
        let at = Vec3::from(*positions.get(corner)?);
        *placed = match skin {
            Some((skin, bindposes)) => skinned(mesh, corner, at, skin, bindposes, joint)?,
            None => global.transform_point(at),
        };
    }
    let point = corners
        .iter()
        .zip(sample.weights)
        .map(|(c, w)| *c * w)
        .sum();
    let normal = (corners[1] - corners[0])
        .cross(corners[2] - corners[0])
        .normalize_or_zero();
    Some((point, normal))
}

fn skinned(
    mesh: &Mesh,
    vertex: usize,
    at: Vec3,
    skin: &SkinnedMesh,
    bindposes: &SkinnedMeshInverseBindposes,
    joint_at: &dyn Fn(Entity) -> Option<Mat4>,
) -> Option<Vec3> {
    let Some(VertexAttributeValues::Uint16x4(indices)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        return None;
    };
    let Some(VertexAttributeValues::Float32x4(weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        return None;
    };
    let mut placed = Vec3::ZERO;
    for (joint, weight) in indices.get(vertex)?.iter().zip(weights.get(vertex)?) {
        if *weight <= 0.0 {
            continue;
        }
        let joint = *joint as usize;
        let world = joint_at(*skin.joints.get(joint)?)?;
        placed += (world * *bindposes.get(joint)?).transform_point3(at) * *weight;
    }
    Some(placed)
}

fn positions(mesh: &Mesh) -> Option<&Vec<[f32; 3]>> {
    match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(p) => Some(p),
        _ => None,
    }
}

/// A fixed spread in [0, 1) for the `k`th sample's `axis`, the same on every run.
fn hash(k: usize, axis: u32) -> f32 {
    let mut x = (k as u32).wrapping_mul(0x9E37_79B9) ^ axis.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}
