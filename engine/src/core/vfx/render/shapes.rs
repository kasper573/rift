//! The shapes mesh layers are drawn on, and the meshes particles are drawn as. Each shape has uvs
//! a shader can read as directions: u across the sweep (round, or along a path), v across the
//! width (outward, up, or round a path).

use std::f32::consts::PI;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::super::MeshShape;

/// The mesh a shape is drawn on, for the shapes made of their measures alone: none for one drawn
/// along a path.
pub fn build(shape: &MeshShape) -> Option<Mesh> {
    match *shape {
        MeshShape::Band {
            angle,
            inner,
            outer,
        } => Some(band(angle.0, inner.0, outer.0)),
        MeshShape::Tube {
            angle,
            bottom,
            top,
            height,
            twist,
            sides,
        } => Some(prism(angle.0, bottom.0, top.0, height.0, twist.0, sides)),
        MeshShape::Sphere { angle, from, to } => Some(sphere(angle.0, from.0, to.0)),
        MeshShape::Jaw {
            radius,
            angle,
            teeth,
        } => Some(jaw(radius.0, angle.0, teeth)),
        MeshShape::Burst {
            spikes,
            inner,
            outer,
        } => Some(burst(spikes, inner.0, outer.0)),
        MeshShape::PathTube { .. } => None,
    }
}

/// The tubes of a bolt through `path`: the main one jagged by `jitter`, and `branches` forks off
/// each stretch, drawn anew for each `seed`.
pub fn bolts(path: &[Vec3], radius: f32, jitter: f32, branches: f32, seed: u64) -> Vec<Mesh> {
    let mut rng = oorandom::Rand32::new(seed);
    let mut signed = || rng.rand_float() * 2.0 - 1.0;
    let mut tubes = Vec::new();
    let mut main = Vec::new();
    let mut radii = Vec::new();
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = a.distance(b);
        let steps = ((length / BOLT_STEP).ceil() as usize).max(3);
        let ahead = (b - a).normalize_or(Vec3::X);
        let side = ahead.cross(Vec3::Y).normalize_or(Vec3::Z);
        let up = side.cross(ahead);
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let taper = (t * PI).sin();
            let off = (side * signed() + up * signed()) * jitter * taper;
            main.push(a.lerp(b, t) + off);
            radii.push(radius * (0.6 + 0.4 * taper));
            if s > 0 && (signed() * 0.5 + 0.5) < branches / steps as f32 {
                let mut fork = vec![main[main.len() - 1]];
                let mut r = vec![radius * 0.6];
                let way = (ahead + side * signed() * 1.2 + up * signed() * 0.8).normalize();
                let n = 3 + ((signed() * 0.5 + 0.5) * 3.0) as usize;
                for k in 1..=n {
                    let prev = fork[k - 1];
                    fork.push(
                        prev + way * BOLT_STEP + (side * signed() + up * signed()) * jitter * 0.6,
                    );
                    r.push(radius * 0.6 * (1.0 - k as f32 / n as f32) + 0.005);
                }
                tubes.push(path_tube(&fork, &r, 5));
            }
        }
    }
    if let Some(last) = path.last() {
        main.push(*last);
        radii.push(radius * 0.6);
    }
    if main.len() >= 2 {
        tubes.insert(0, path_tube(&main, &radii, 6));
    }
    tubes
}

/// How far apart the kinks of a bolt are, in metres.
const BOLT_STEP: f32 = 0.22;

pub fn band(angle: f32, inner: f32, outer: f32) -> Mesh {
    let around = ((angle / 3.0).ceil() as usize).max(8);
    let across = 6;
    grid(around, across, |u, v| {
        let a = (u - 0.5) * angle.to_radians();
        let r = inner + (outer - inner) * v;
        (Vec3::new(a.sin() * r, 0.0, a.cos() * r), Vec3::Y)
    })
}

pub fn tube(angle: f32, bottom: f32, top: f32, height: f32, twist: f32) -> Mesh {
    let around = ((angle / 6.0).ceil() as usize).max(8);
    let up = 16;
    grid(around, up, |u, v| {
        let a = (u - 0.5) * angle.to_radians() + twist.to_radians() * v;
        let r = bottom + (top - bottom) * v;
        let out = Vec3::new(a.sin(), 0.0, a.cos());
        let slope = (bottom - top) / height.max(1e-3);
        (
            out * r + Vec3::Y * height * v,
            (out + Vec3::Y * slope).normalize(),
        )
    })
}

/// A tube whose round is `sides` flat faces, corners on the radius; round when 0.
pub fn prism(angle: f32, bottom: f32, top: f32, height: f32, twist: f32, sides: u32) -> Mesh {
    if sides == 0 {
        return tube(angle, bottom, top, height, twist);
    }
    let per = 8usize;
    let around = sides as usize * per;
    let step = std::f32::consts::TAU / sides as f32;
    let corner = |k: usize| {
        let a = k as f32 * step + step / 2.0;
        Vec3::new(a.sin(), 0.0, a.cos())
    };
    grid(around, 16, |u, v| {
        let at = u * sides as f32;
        let k = (at.floor() as usize).min(sides as usize - 1);
        let f = at - k as f32;
        let (c0, c1) = (corner(k), corner(k + 1));
        let r = bottom + (top - bottom) * v;
        let mid = (k as f32 + 1.0) * step;
        (
            c0.lerp(c1, f) * r + Vec3::Y * height * v,
            Vec3::new(mid.sin(), 0.0, mid.cos()),
        )
    })
}

pub fn sphere(angle: f32, from: f32, to: f32) -> Mesh {
    let around = ((angle / 6.0).ceil() as usize).max(8);
    let up = ((to - from) / 6.0).ceil().max(4.0) as usize;
    grid(around, up, |u, v| {
        let lon = (u - 0.5) * angle.to_radians();
        let lat = (from + (to - from) * v).to_radians();
        let p = Vec3::new(lon.sin() * lat.cos(), lat.sin(), lon.cos() * lat.cos());
        (p, p)
    })
}

/// A tube through `points`, `radius` thick, `sides` round.
pub fn path_tube(points: &[Vec3], radius: &[f32], sides: usize) -> Mesh {
    let n = points.len();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut length = vec![0.0f32; n];
    for i in 1..n {
        length[i] = length[i - 1] + points[i].distance(points[i - 1]);
    }
    let total = length[n - 1].max(1e-4);
    for i in 0..n {
        let ahead = if i + 1 < n {
            points[i + 1] - points[i]
        } else {
            points[i] - points[i - 1]
        }
        .normalize_or(Vec3::X);
        let side = ahead.cross(Vec3::Y).normalize_or(Vec3::Z);
        let up = side.cross(ahead);
        for s in 0..=sides {
            let a = s as f32 / sides as f32 * std::f32::consts::TAU;
            let normal = side * a.cos() + up * a.sin();
            positions.push((points[i] + normal * radius[i]).to_array());
            normals.push(normal.to_array());
            uvs.push([length[i] / total, s as f32 / sides as f32]);
        }
    }
    let row = sides + 1;
    let mut indices = Vec::new();
    for i in 0..n - 1 {
        for s in 0..sides {
            let a = (i * row + s) as u32;
            let b = a + row as u32;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    finish(positions, normals, uvs, indices)
}

pub fn ico(subdivisions: u32) -> Mesh {
    Sphere::new(0.5)
        .mesh()
        .ico(subdivisions)
        .expect("an icosphere this fine")
}

/// A long double pyramid, its point along +x: a shard of stone or glass.
pub fn shard() -> Mesh {
    let tips = [Vec3::new(0.6, 0.0, 0.0), Vec3::new(-0.4, 0.0, 0.0)];
    let ring: Vec<Vec3> = (0..4)
        .map(|i| {
            let a = i as f32 / 4.0 * std::f32::consts::TAU + 0.3;
            Vec3::new(0.05, a.cos() * 0.22, a.sin() * 0.16)
        })
        .collect();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (t, tip) in tips.iter().enumerate() {
        for i in 0..4 {
            let (a, b) = (ring[i], ring[(i + 1) % 4]);
            let (a, b) = if t == 0 { (a, b) } else { (b, a) };
            let normal = (a - *tip).cross(b - *tip).normalize();
            for p in [*tip, a, b] {
                positions.push(p.to_array());
                normals.push(normal.to_array());
                uvs.push([t as f32, i as f32 / 4.0]);
            }
        }
    }
    let indices = (0..positions.len() as u32).collect();
    finish(positions, normals, uvs, indices)
}

/// An upper jaw, as `Shape::Jaw`: u runs along the ridge, and down each tooth to its tip.
pub fn jaw(radius: f32, angle: f32, teeth: u32) -> Mesh {
    let centre = Vec3::new(0.0, 0.0, radius * 1.25);
    let half = angle.to_radians() / 2.0;
    let along = |u: f32| {
        let a = (u - 0.5) * 2.0 * half;
        (
            centre + Vec3::new(a.sin(), 0.0, a.cos()) * radius,
            Vec3::new(a.sin(), 0.0, a.cos()),
        )
    };
    let mut parts = Vec::new();
    let ridge: Vec<Vec3> = (0..=48).map(|k| along(k as f32 / 48.0).0).collect();
    let thick: Vec<f32> = (0..=48)
        .map(|k| {
            let u = k as f32 / 48.0;
            radius * 0.16 * (0.55 + 0.45 * (u * PI).sin())
        })
        .collect();
    parts.push(path_tube(&ridge, &thick, 8));
    for k in 0..teeth {
        let u = (k as f32 + 0.5) / teeth as f32;
        let (base, out) = along(u);
        let from_front = ((u - 0.5) * 2.0).abs();
        let fang = (-(from_front - 0.55).powi(2) / 0.01).exp();
        let length = radius * (0.3 + 0.42 * fang) * (1.0 - 0.3 * from_front);
        let width = radius * (0.1 + 0.05 * fang);
        let tip = base - Vec3::Y * length - out * length * 0.18;
        let mut points = Vec::new();
        let mut radii = Vec::new();
        for s in 0..=6 {
            let t = s as f32 / 6.0;
            let bend = out * (t * t) * length * 0.12;
            points.push(base.lerp(tip, t) - bend);
            radii.push(width * (1.0 - t).powf(0.8) + 0.002);
        }
        parts.push(path_tube(&points, &radii, 6));
    }
    merge(parts)
}

/// A starburst, as `Shape::Burst`: u runs out along each spike to its tip.
pub fn burst(spikes: u32, inner: f32, outer: f32) -> Mesh {
    let mut parts = vec![ico(1).scaled_by(Vec3::splat(inner * 2.0))];
    let golden = PI * (3.0 - 5f32.sqrt());
    for k in 0..spikes {
        let y = 1.0 - 2.0 * (k as f32 + 0.5) / spikes as f32;
        let r = (1.0 - y * y).sqrt();
        let a = k as f32 * golden;
        let jitter = ((k as f32 * 12.9898).sin() * 43758.547).fract();
        let way = Vec3::new(a.cos() * r, y, a.sin() * r).normalize();
        let length = inner + (outer - inner) * (0.55 + 0.45 * jitter);
        let points = [Vec3::ZERO, way * length];
        parts.push(path_tube(&points, &[inner * 0.55, 0.002], 5));
    }
    merge(parts)
}

/// One mesh of several, each keeping its own uvs.
fn merge(parts: Vec<Mesh>) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices = Vec::new();
    for part in parts {
        let at = positions.len() as u32;
        let read3 = |id| match part.attribute(id) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => Vec::new(),
        };
        let p = read3(Mesh::ATTRIBUTE_POSITION);
        let n = read3(Mesh::ATTRIBUTE_NORMAL);
        let uv = match part.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
            _ => vec![[0.0, 0.0]; p.len()],
        };
        let count = p.len();
        positions.extend(p);
        normals.extend(n);
        uvs.extend(uv);
        match part.indices() {
            Some(found) => indices.extend(found.iter().map(|i| i as u32 + at)),
            None => indices.extend((0..count as u32).map(|i| i + at)),
        }
    }
    finish(positions, normals, uvs, indices)
}

/// A thorn laid along +x, from its base at the origin to its point a unit out.
pub fn spike() -> Mesh {
    tube(360.0, 0.16, 0.0, 1.0, 0.0).rotated_by(Quat::from_rotation_z(-PI / 2.0))
}

/// A model's mesh as a particle is drawn as, laid along +x, with the uvs and normals the
/// particles' looks read and nothing else.
pub fn laid_model(mesh: &Mesh) -> Mesh {
    let mut laid = mesh.clone().rotated_by(Quat::from_rotation_z(-PI / 2.0));
    if laid.attribute(Mesh::ATTRIBUTE_UV_0).is_none() {
        let count = laid.count_vertices();
        laid.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5f32, 0.5]; count]);
    }
    if laid.attribute(Mesh::ATTRIBUTE_NORMAL).is_none() {
        laid.compute_normals();
    }
    for id in [
        Mesh::ATTRIBUTE_COLOR.id,
        Mesh::ATTRIBUTE_TANGENT.id,
        Mesh::ATTRIBUTE_UV_1.id,
    ] {
        laid.remove_attribute(id);
    }
    laid
}

fn grid(around: usize, across: usize, at: impl Fn(f32, f32) -> (Vec3, Vec3)) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..=across {
        for i in 0..=around {
            let (u, v) = (i as f32 / around as f32, j as f32 / across as f32);
            let (p, n) = at(u, v);
            positions.push(p.to_array());
            normals.push(n.normalize_or(Vec3::Y).to_array());
            uvs.push([u, v]);
        }
    }
    let row = (around + 1) as u32;
    let mut indices = Vec::new();
    for j in 0..across as u32 {
        for i in 0..around as u32 {
            let a = j * row + i;
            indices.extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    finish(positions, normals, uvs, indices)
}

fn finish(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}
