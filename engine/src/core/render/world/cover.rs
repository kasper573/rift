//! The geometry a ground's cover is laid out as, from the paint: a sheet over the painted cells,
//! blades standing up from the terrain, or copies of a prop strewn over it. Every vertex carries,
//! in its second uv, how far up the geometry it stands (0 at the root, 1 at the top) and how
//! heavily the material is painted there, for the shader that moves and paints it.

use bevy::asset::RenderAssetUsages;
use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use crate::core::math::{Pos, Rng};
use crate::core::terrain::{Heightfield, SAMPLES_PER_METER};
use crate::core::units::{GROUND_ORIGIN, Meters};

/// Cells of paint along a chunk's side; a chunk is one mesh, culled as one.
pub const CHUNK_CELLS: u32 = 16;
/// How far above the terrain a sheet lies, so it draws over the ground without fighting it.
pub const SHEET_LIFT: f32 = 0.02;

/// A chunk of the paint: `columns` and `rows` of half-metre cells from the cell at `first`, each
/// painted `share(column, row)` of the material, 0 to 1.
pub struct Patch<'a> {
    pub heights: &'a Heightfield,
    pub first: (u32, u32),
    pub columns: u32,
    pub rows: u32,
    pub share: &'a dyn Fn(u32, u32) -> f32,
}

/// Blades: how many a square metre grows at full weight and how tall they stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blades {
    pub per_square_meter: f32,
    pub height: f32,
    pub width: f32,
}

/// Copies of a prop: how many a square metre holds at full weight and how large they stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strew {
    pub per_square_meter: f32,
    pub scale: f32,
}

/// A piece of a prop's model as it is copied: its mesh, placed by its node's transform.
pub struct Piece {
    pub mesh: Mesh,
    pub transform: Transform,
}

/// Where a strewn copy of a prop stands, how heavily its cell is painted and the seed it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub transform: Transform,
    pub weight: f32,
    pub seed: f32,
}

/// A chunk waiting for the prop it strews to load: where every copy stands.
#[derive(Component)]
pub struct Strewing {
    pub gltf: Handle<Gltf>,
    pub places: Vec<Place>,
}

/// What a cover's vertex tells its shader beside its place: `measure`, how far up its blade or
/// copy it stands (0 at the root, 1 at the top) or, on a sheet at a level, how many metres of
/// ground lie under it; `weight`, how heavily its cell is painted; and `seed`, a number of its own
/// in 0..1, alike over a blade or a copy and 0 on a sheet, so a shader can vary one from the next.
/// Carried in the mesh's second uv: the measure in x, and in y the weight in the fraction with the
/// seed in whole steps, which stays exact because no triangle mixes two seeds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub measure: f32,
    pub weight: f32,
    pub seed: f32,
}

const SEEDS: f32 = 1023.0;

impl Mark {
    pub fn of(uv: [f32; 2]) -> Mark {
        Mark {
            measure: uv[0],
            weight: uv[1].fract(),
            seed: uv[1].floor() / SEEDS,
        }
    }

    fn pack(self) -> [f32; 2] {
        let weight = self.weight.clamp(0.0, 1.0 - 1.0 / (SEEDS + 1.0));
        [
            self.measure,
            weight + (self.seed.clamp(0.0, 1.0) * SEEDS).floor(),
        ]
    }
}

/// A sheet over the patch's painted cells: a quad per cell, each vertex weighted by how heavily
/// the cells around it are painted, lying a hair over the ground or, at a `level`, flat at that
/// height with the depth of ground beneath each vertex. Its uv runs over the whole area, so a
/// shader's pattern crosses from chunk to chunk unbroken. None when nothing is painted.
pub fn sheet(patch: &Patch, level: Option<f32>) -> Option<Mesh> {
    let (columns, rows) = (patch.columns, patch.rows);
    let (area_columns, area_rows) = patch.heights.samples();
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let weight_at = |column: u32, row: u32| -> f32 {
        let mut total = 0.0;
        let mut count = 0.0;
        for (dc, dr) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            if column + dc >= 1 && row + dr >= 1 {
                let (c, r) = (column + dc - 1, row + dr - 1);
                if c >= patch.first.0
                    && c < patch.first.0 + columns
                    && r >= patch.first.1
                    && r < patch.first.1 + rows
                {
                    total += (patch.share)(c, r).clamp(0.0, 1.0);
                    count += 1.0;
                }
            }
        }
        if count > 0.0 { total / count } else { 0.0 }
    };
    let mut any = false;
    let mut positions = Vec::with_capacity(((columns + 1) * (rows + 1)) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut tips = Vec::with_capacity(positions.capacity());
    for row in patch.first.1..=patch.first.1 + rows {
        for column in patch.first.0..=patch.first.0 + columns {
            let at = Pos::new(
                GROUND_ORIGIN.x + column as f32 * step,
                GROUND_ORIGIN.y + row as f32 * step,
            );
            let weight = weight_at(column, row);
            any |= weight > 0.0;
            let ground = patch.heights.height_at(at).0;
            match level {
                Some(level) => {
                    positions.push([at.x, level, at.y]);
                    normals.push([0.0, 1.0, 0.0]);
                }
                None => {
                    positions.push([at.x, ground + SHEET_LIFT, at.y]);
                    normals.push(ground_normal(patch.heights, at));
                }
            }
            uvs.push([
                column as f32 / (area_columns - 1).max(1) as f32,
                row as f32 / (area_rows - 1).max(1) as f32,
            ]);
            tips.push(
                Mark {
                    measure: level.map_or(0.0, |level| (level - ground).max(0.0)),
                    weight,
                    seed: 0.0,
                }
                .pack(),
            );
        }
    }
    if !any {
        return None;
    }
    let across = columns + 1;
    let mut indices = Vec::with_capacity((columns * rows * 6) as usize);
    for row in 0..rows {
        for column in 0..columns {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    Some(assemble(positions, normals, uvs, tips, None, indices))
}

/// The patch's blades: each cell grows its share of the density, a triangle per blade whose uv
/// runs across and up, scattered the same way for the same `seed`. None when nothing grows.
pub fn blades(patch: &Patch, blades: &Blades, seed: u64) -> Option<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut tips = Vec::new();
    let mut rng = Rng::new(seed);
    for (at, weight) in places(patch, blades.per_square_meter, &mut rng) {
        let ground = patch.heights.height_at(at).0;
        let yaw = rng.rand_float() * std::f32::consts::TAU;
        let height = blades.height * (0.7 + 0.6 * rng.rand_float());
        let half = blades.width / 2.0 * (0.8 + 0.4 * rng.rand_float());
        let (sin, cos) = yaw.sin_cos();
        let across = Vec3::new(cos * half, 0.0, sin * half);
        let root = Vec3::new(at.x, ground, at.y);
        positions.push((root - across).to_array());
        positions.push((root + across).to_array());
        positions.push((root + Vec3::Y * height).to_array());
        normals.extend([ground_normal(patch.heights, at); 3]);
        uvs.extend([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        let seed = rng.rand_float();
        let mark = |measure| {
            Mark {
                measure,
                weight,
                seed,
            }
            .pack()
        };
        tips.extend([mark(0.0), mark(0.0), mark(1.0)]);
    }
    if positions.is_empty() {
        return None;
    }
    let indices = (0..positions.len() as u32).collect();
    Some(assemble(positions, normals, uvs, tips, None, indices))
}

/// Where the patch's strewn copies stand, each cell holding its share of the density, turned any
/// way and about `scale` large, scattered the same way for the same `seed`, each with how heavily
/// its cell is painted and a seed of its own. Empty when nothing is strewn.
pub fn strew_places(patch: &Patch, strew: &Strew, seed: u64) -> Vec<Place> {
    let mut rng = Rng::new(seed);
    places(patch, strew.per_square_meter, &mut rng)
        .into_iter()
        .map(|(at, weight)| {
            let ground = patch.heights.height_at(at).0;
            let yaw = rng.rand_float() * std::f32::consts::TAU;
            let scale = strew.scale * (0.8 + 0.4 * rng.rand_float());
            Place {
                transform: Transform {
                    translation: Vec3::new(at.x, ground, at.y),
                    rotation: Quat::from_rotation_y(yaw),
                    scale: Vec3::splat(scale),
                },
                weight,
                seed: rng.rand_float(),
            }
        })
        .collect()
}

/// The `pieces` of a prop copied to every place: positions and normals carried through the
/// piece's and the copy's transforms, the piece's uv kept, its colours kept when every piece has
/// them, and every vertex marked how far up the prop it stands. None when there is nothing to
/// copy.
pub fn strewn(pieces: &[Piece], places: &[Place]) -> Option<Mesh> {
    let placed: Vec<PieceVertices> = pieces.iter().filter_map(piece_vertices).collect();
    if placed.is_empty() || places.is_empty() {
        return None;
    }
    let height = placed
        .iter()
        .flat_map(|piece| piece.positions.iter().map(|position| position.y))
        .fold(0.0f32, f32::max)
        .max(1e-3);
    let colored = placed.iter().all(|piece| piece.colors.is_some());
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut tips = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for place in places {
        for piece in &placed {
            let base = positions.len() as u32;
            for (position, normal) in piece.positions.iter().zip(&piece.normals) {
                positions.push(place.transform.transform_point(*position).to_array());
                normals.push(
                    (place.transform.rotation * *normal)
                        .normalize_or_zero()
                        .to_array(),
                );
                tips.push(
                    Mark {
                        measure: (position.y / height).clamp(0.0, 1.0),
                        weight: place.weight,
                        seed: place.seed,
                    }
                    .pack(),
                );
            }
            uvs.extend(piece.uvs.iter().copied());
            if colored && let Some(piece_colors) = &piece.colors {
                colors.extend(piece_colors.iter().copied());
            }
            indices.extend(piece.indices.iter().map(|index| base + index));
        }
    }
    Some(assemble(
        positions,
        normals,
        uvs,
        tips,
        colored.then_some(colors),
        indices,
    ))
}

/// A chunk whose prop has loaded gets its mesh and keeps its material.
pub(super) fn strew_loaded(
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    waiting: Query<(Entity, &Strewing)>,
    mut commands: Commands,
) {
    for (entity, strewing) in &waiting {
        let Some(gltf) = gltfs.get(&strewing.gltf) else {
            continue;
        };
        let Some(pieces) = pieces_of(gltf, &nodes, &gltf_meshes, &meshes) else {
            continue;
        };
        let mut chunk = commands.entity(entity);
        chunk.remove::<Strewing>();
        if let Some(mesh) = strewn(&pieces, &strewing.places) {
            chunk.insert(Mesh3d(meshes.add(mesh)));
        }
    }
}

/// A piece's vertices in the prop's own space, with the colours it carries if any.
struct PieceVertices {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    colors: Option<Vec<[f32; 4]>>,
    indices: Vec<u32>,
}

/// A cell's places for `per_square_meter` of something at full weight, with the weight each
/// stands in: jittered within the cell, as many as the weight's share, the remainder by chance.
fn places(patch: &Patch, per_square_meter: f32, rng: &mut Rng) -> Vec<(Pos<Meters>, f32)> {
    let cell = 1.0 / SAMPLES_PER_METER as f32;
    let per_cell = per_square_meter * cell * cell;
    let mut placed = Vec::new();
    for row in patch.first.1..patch.first.1 + patch.rows {
        for column in patch.first.0..patch.first.0 + patch.columns {
            let weight = (patch.share)(column, row).clamp(0.0, 1.0);
            let wanted = per_cell * weight;
            let count = wanted.floor() as u32 + u32::from(rng.rand_float() < wanted.fract());
            for _ in 0..count {
                placed.push((
                    Pos::new(
                        GROUND_ORIGIN.x + (column as f32 + rng.rand_float()) * cell,
                        GROUND_ORIGIN.y + (row as f32 + rng.rand_float()) * cell,
                    ),
                    weight,
                ));
            }
        }
    }
    placed
}

/// The prop's meshes as pieces, each placed by its node's transform down from the roots, once
/// every mesh has loaded.
fn pieces_of(
    gltf: &Gltf,
    nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Option<Vec<Piece>> {
    let children: HashSet<AssetId<GltfNode>> = gltf
        .nodes
        .iter()
        .filter_map(|node| nodes.get(node))
        .flat_map(|node| node.children.iter().map(Handle::id))
        .collect();
    let mut pieces = Vec::new();
    let mut pending: Vec<(Handle<GltfNode>, Transform)> = gltf
        .nodes
        .iter()
        .filter(|node| !children.contains(&node.id()))
        .map(|node| (node.clone(), Transform::IDENTITY))
        .collect();
    while let Some((handle, parent)) = pending.pop() {
        let node = nodes.get(&handle)?;
        let transform = parent * node.transform;
        if let Some(mesh) = &node.mesh {
            for primitive in &gltf_meshes.get(mesh)?.primitives {
                pieces.push(Piece {
                    mesh: meshes.get(&primitive.mesh)?.clone(),
                    transform,
                });
            }
        }
        pending.extend(node.children.iter().map(|child| (child.clone(), transform)));
    }
    Some(pieces)
}

fn piece_vertices(piece: &Piece) -> Option<PieceVertices> {
    let positions: Vec<Vec3> = piece
        .mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)?
        .as_float3()?
        .iter()
        .map(|position| piece.transform.transform_point(Vec3::from_array(*position)))
        .collect();
    let normals: Vec<Vec3> = match piece.mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(normals) => normals
            .as_float3()?
            .iter()
            .map(|normal| piece.transform.rotation * Vec3::from_array(*normal))
            .collect(),
        None => vec![Vec3::Y; positions.len()],
    };
    let uvs: Vec<[f32; 2]> = match piece.mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uvs)) => uvs.clone(),
        _ => vec![[0.0, 0.0]; positions.len()],
    };
    let colors = match piece.mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(colors)) => Some(colors.clone()),
        _ => None,
    };
    let indices: Vec<u32> = match piece.mesh.indices() {
        Some(indices) => indices.iter().map(|index| index as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    Some(PieceVertices {
        positions,
        normals,
        uvs,
        colors,
        indices,
    })
}

fn assemble(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    tips: Vec<[f32; 2]>,
    colors: Option<Vec<[f32; 4]>>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, tips)
    .with_inserted_indices(Indices::U32(indices));
    if let Some(colors) = colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh
}

fn ground_normal(heights: &Heightfield, at: Pos<Meters>) -> [f32; 3] {
    let step = 1.0 / SAMPLES_PER_METER as f32;
    let east = heights.height_at(Pos::new(at.x + step, at.y)).0
        - heights.height_at(Pos::new(at.x - step, at.y)).0;
    let south = heights.height_at(Pos::new(at.x, at.y + step)).0
        - heights.height_at(Pos::new(at.x, at.y - step)).0;
    Vec3::new(-east, 2.0 * step, -south).normalize().to_array()
}
