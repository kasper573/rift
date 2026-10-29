//! The geometry a ground's cover is laid out as, from the paint: a sheet over the painted cells,
//! blades standing up from the terrain, or copies of a prop strewn over it. Every vertex carries,
//! in its second uv, how far up the geometry it stands (0 at the root, 1 at the top) and how
//! heavily the material is painted there, for the shader that moves and paints it. A strewn
//! copy is an instance of its prop's own mesh, which carries only how far up the prop a vertex
//! stands; the rest of its marks and its tint ride on its tag.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::color::Mix;
use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rng};
use crate::core::units::{GROUND_ORIGIN, Meters};
use crate::core::vfx::cover::CoverMaterial;

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

/// How a piece is spread over the ground: which piece of its cover it is, how many copies a
/// square metre holds at full weight, and the scales and colours each copy is drawn between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub piece: u32,
    pub per_square_meter: f32,
    pub size: (f32, f32),
    pub color: (LinearRgba, LinearRgba),
}

/// A blade's shape at size 1: how wide it stands at its root and how tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BladeShape {
    pub width: f32,
    pub height: f32,
}

/// A piece of a prop's model as it is copied: its mesh, placed by its node's transform, and the
/// material the model draws it with.
pub struct Piece {
    pub mesh: Mesh,
    pub transform: Transform,
    pub material: Option<Handle<GltfMaterial>>,
}

/// Where a strewn copy of a prop stands, how far its tint lies from its spread's first colour to
/// its second (0 to 1), how heavily its cell is painted and the seed it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub transform: Transform,
    pub tint: f32,
    pub weight: f32,
    pub seed: f32,
}

/// A piece of a cover waiting for the prop it strews to load: where every copy stands, chunk by
/// chunk, and the material that draws them.
#[derive(Component)]
pub struct Strewing {
    pub gltf: Handle<Gltf>,
    pub chunks: Vec<Vec<Place>>,
    pub material: Handle<CoverMaterial>,
}

/// What a cover's vertex tells its shader beside its place and colour: `measure`, how far up its
/// blade or copy it stands (0 at the root, 1 at the top) or, on a sheet at a level, how many
/// metres of ground lie under it; `weight`, how heavily its cell is painted; `piece`, which of
/// the cover's pieces it belongs to (0 for a sheet); and `seed`, a number of its own in 0..1,
/// alike over a blade or a copy and 0 on a sheet, so a shader can vary one from the next. Carried
/// in the mesh's second uv: the measure in x, and in y the weight in the fraction with the seed
/// and the piece in whole steps, which stays exact because no triangle mixes two of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub measure: f32,
    pub weight: f32,
    pub piece: u32,
    pub seed: f32,
}

const SEEDS: f32 = 255.0;
const PIECE_STEP: f32 = 256.0;

impl Mark {
    pub fn of(uv: [f32; 2]) -> Mark {
        let steps = uv[1].floor();
        Mark {
            measure: uv[0],
            weight: uv[1].fract(),
            piece: (steps / PIECE_STEP) as u32,
            seed: (steps % PIECE_STEP) / SEEDS,
        }
    }

    fn pack(self) -> [f32; 2] {
        let weight = self.weight.clamp(0.0, 1.0 - 1.0 / (SEEDS + 1.0));
        let steps = seed_steps(self.seed) + self.piece as f32 * PIECE_STEP;
        [self.measure, weight + steps]
    }
}

impl Place {
    /// The copy's tag: its weight, its seed and its tint, a byte each from the lowest, which the
    /// cover header turns into the marks and the colour a sheet's or a blade's vertices carry.
    pub fn tag(&self) -> MeshTag {
        let byte = |share: f32| (share.clamp(0.0, 1.0) * 255.0).round() as u32;
        MeshTag::new(
            byte(self.weight) | (seed_steps(self.seed) as u32) << 8 | byte(self.tint) << 16,
        )
    }
}

fn seed_steps(seed: f32) -> f32 {
    (seed.clamp(0.0, 1.0) * SEEDS).floor()
}

/// A copy's own draw from its spread: its scale, and how far its colour lies between the spread's.
fn draw(spread: &Spread, rng: &mut Rng) -> (f32, f32) {
    let size = spread.size.0 + (spread.size.1 - spread.size.0) * rng.rand_float();
    (size, rng.rand_float())
}

/// A sheet over the patch's painted cells: a quad per cell, each vertex weighted by how heavily
/// the cells around it are painted, lying a hair over the ground or, at a `level`, flat at that
/// height with the depth of ground beneath each vertex. Its uv runs over the whole area, so a
/// shader's pattern crosses from chunk to chunk unbroken. None when nothing is painted.
pub fn sheet(patch: &Patch, level: Option<f32>, color: LinearRgba) -> Option<Mesh> {
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
                    piece: 0,
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
    let colors = vec![color.to_f32_array(); positions.len()];
    Some(assemble(
        positions,
        normals,
        uvs,
        tips,
        Some(colors),
        indices,
    ))
}

/// The patch's blades: each cell grows its share of the spread, a triangle per blade of the
/// `shape` at its own size and colour, its uv running across and up, scattered the same way for
/// the same `seed`. None when nothing grows.
pub fn blades(patch: &Patch, shape: &BladeShape, spread: &Spread, seed: u64) -> Option<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut tips = Vec::new();
    let mut colors = Vec::new();
    let mut rng = Rng::new(seed);
    for (at, weight) in places(patch, spread.per_square_meter, &mut rng) {
        let ground = patch.heights.height_at(at).0;
        let yaw = rng.rand_float() * std::f32::consts::TAU;
        let (size, tint) = draw(spread, &mut rng);
        let color = spread.color.0.mix(&spread.color.1, tint);
        let height = shape.height * size;
        let half = shape.width / 2.0 * size;
        let (sin, cos) = yaw.sin_cos();
        let across = Vec3::new(cos * half, 0.0, sin * half);
        let root = Vec3::new(at.x, ground, at.y);
        positions.push((root - across).to_array());
        positions.push((root + across).to_array());
        positions.push((root + Vec3::Y * height).to_array());
        normals.extend([ground_normal(patch.heights, at); 3]);
        uvs.extend([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        colors.extend([color.to_f32_array(); 3]);
        let seed = rng.rand_float();
        let mark = |measure| {
            Mark {
                measure,
                weight,
                piece: spread.piece,
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
    Some(assemble(
        positions,
        normals,
        uvs,
        tips,
        Some(colors),
        indices,
    ))
}

/// Where the patch's strewn copies stand, each cell holding its share of the spread, turned any
/// way, each at its own size and tint, scattered the same way for the same `seed`, with how
/// heavily its cell is painted and a seed of its own. Empty when nothing is strewn.
pub fn strew_places(patch: &Patch, spread: &Spread, seed: u64) -> Vec<Place> {
    let mut rng = Rng::new(seed);
    places(patch, spread.per_square_meter, &mut rng)
        .into_iter()
        .map(|(at, weight)| {
            let ground = patch.heights.height_at(at).0;
            let yaw = rng.rand_float() * std::f32::consts::TAU;
            let (size, tint) = draw(spread, &mut rng);
            Place {
                transform: Transform {
                    translation: Vec3::new(at.x, ground, at.y),
                    rotation: Quat::from_rotation_y(yaw),
                    scale: Vec3::splat(size),
                },
                tint,
                weight,
                seed: rng.rand_float(),
            }
        })
        .collect()
}

/// The meshes a strewn copy of a prop is drawn with, one for each material the model draws its
/// `pieces` with: positions and normals carried through the piece's transform, the piece's uv and
/// colours kept (white where it has none, so a copy's tint has colours to fall on), and every
/// vertex marked how far up the whole prop it stands. Empty when there is nothing to copy.
pub fn copy_meshes(pieces: &[Piece]) -> Vec<(Option<Handle<GltfMaterial>>, Mesh)> {
    let placed: Vec<(&Piece, PieceVertices)> = pieces
        .iter()
        .filter_map(|piece| Some((piece, piece_vertices(piece)?)))
        .collect();
    let height = placed
        .iter()
        .flat_map(|(_, piece)| piece.positions.iter().map(|position| position.y))
        .fold(0.0f32, f32::max)
        .max(1e-3);
    let mut materials: Vec<Option<Handle<GltfMaterial>>> = Vec::new();
    for (piece, _) in &placed {
        if !materials.contains(&piece.material) {
            materials.push(piece.material.clone());
        }
    }
    materials
        .into_iter()
        .map(|material| {
            let drawn: Vec<&PieceVertices> = placed
                .iter()
                .filter(|(piece, _)| piece.material == material)
                .map(|(_, vertices)| vertices)
                .collect();
            (material, merged(&drawn, height))
        })
        .collect()
}

/// The pieces as one mesh, each vertex marked how far up `height` it stands.
fn merged(pieces: &[&PieceVertices], height: f32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut tips = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for piece in pieces {
        let base = positions.len() as u32;
        positions.extend(piece.positions.iter().map(|position| position.to_array()));
        normals.extend(piece.normals.iter().map(|normal| normal.to_array()));
        tips.extend(piece.positions.iter().map(|position| {
            Mark {
                measure: (position.y / height).clamp(0.0, 1.0),
                weight: 0.0,
                piece: 0,
                seed: 0.0,
            }
            .pack()
        }));
        uvs.extend(piece.uvs.iter().copied());
        match &piece.colors {
            Some(piece_colors) => colors.extend(piece_colors.iter().copied()),
            None => colors.extend(std::iter::repeat_n([1.0; 4], piece.positions.len())),
        }
        indices.extend(piece.indices.iter().map(|index| base + index));
    }
    assemble(positions, normals, uvs, tips, Some(colors), indices)
}

/// The cover materials standing in for a piece's own under each texture a prop's material draws
/// with, shared by every piece alike.
#[derive(Resource, Default)]
pub(super) struct TexturedCovers(
    HashMap<(AssetId<CoverMaterial>, AssetId<Image>), Handle<CoverMaterial>>,
);

/// The meshes each loaded prop's copies are drawn with, made once however many covers strew it.
#[derive(Resource, Default)]
pub(super) struct CopyMeshes(HashMap<AssetId<Gltf>, Vec<CopyMesh>>);

/// A mesh a prop's copies are drawn with, the material of the prop's it is drawn in, and its
/// bounds, measured as it is made since the mesh leaves the main world once it is drawn.
#[derive(Clone)]
struct CopyMesh {
    material: Option<Handle<GltfMaterial>>,
    mesh: Handle<Mesh>,
    bounds: Aabb,
}

/// Forgets the textured cover materials made for cover materials that are gone, so the pictures
/// they draw with go with them, and the meshes made from props that are gone.
pub(super) fn forget_dropped(
    mut covers: MessageReader<AssetEvent<CoverMaterial>>,
    mut props: MessageReader<AssetEvent<Gltf>>,
    mut textured: ResMut<TexturedCovers>,
    mut copies: ResMut<CopyMeshes>,
) {
    let gone = super::dropped(&mut covers);
    if !gone.is_empty() {
        textured.0.retain(|(cover, _), _| !gone.contains(cover));
    }
    for prop in super::dropped(&mut props) {
        copies.0.remove(&prop);
    }
}

/// The assets a strewn prop is drawn from.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct StrewnAssets<'w> {
    gltfs: Res<'w, Assets<Gltf>>,
    nodes: Res<'w, Assets<GltfNode>>,
    gltf_meshes: Res<'w, Assets<GltfMesh>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    gltf_materials: Res<'w, Assets<GltfMaterial>>,
    covers: ResMut<'w, Assets<CoverMaterial>>,
    textured: ResMut<'w, TexturedCovers>,
    copies: ResMut<'w, CopyMeshes>,
}

/// A piece whose prop has loaded gets a copy of the prop at every place, an instance of each of
/// the prop's meshes drawn with the piece's material in that mesh's texture, so a copy keeps the
/// shapes its texture cuts out.
pub(super) fn strew_loaded(
    mut assets: StrewnAssets,
    waiting: Query<(Entity, &Strewing)>,
    mut commands: Commands,
) {
    for (entity, strewing) in &waiting {
        let Some(meshes) = assets.copy_meshes(&strewing.gltf) else {
            continue;
        };
        commands.entity(entity).remove::<Strewing>();
        for CopyMesh {
            material,
            mesh,
            bounds,
        } in meshes
        {
            let texture = material.and_then(|material| {
                assets
                    .gltf_materials
                    .get(&material)?
                    .base_color_texture
                    .clone()
            });
            let drawn = match texture {
                Some(texture) => assets.textured_cover(&strewing.material, texture),
                None => strewing.material.clone(),
            };
            // Copies hang under a node per chunk rather than straight under the piece: a node
            // whose transform is set has all its children visited, and the area's root is set
            // whenever anything under it moves, so a piece's thousands of copies would otherwise
            // be walked every frame.
            for places in &strewing.chunks {
                let chunk = commands
                    .spawn((Transform::default(), Visibility::default(), ChildOf(entity)))
                    .id();
                let copies: Vec<_> = places
                    .iter()
                    .map(|place| {
                        (
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(drawn.clone()),
                            place.transform,
                            place.tag(),
                            bounds,
                            NotShadowCaster,
                            ChildOf(chunk),
                        )
                    })
                    .collect();
                commands.spawn_batch(copies);
            }
        }
    }
}

impl StrewnAssets<'_> {
    fn copy_meshes(&mut self, gltf: &Handle<Gltf>) -> Option<Vec<CopyMesh>> {
        if let Some(made) = self.copies.0.get(&gltf.id()) {
            return Some(made.clone());
        }
        let pieces = pieces_of(
            self.gltfs.get(gltf)?,
            &self.nodes,
            &self.gltf_meshes,
            &self.meshes,
        )?;
        let made: Vec<CopyMesh> = copy_meshes(&pieces)
            .into_iter()
            .filter_map(|(material, mesh)| {
                Some(CopyMesh {
                    material,
                    bounds: mesh.get_aabb()?,
                    mesh: self.meshes.add(mesh),
                })
            })
            .collect();
        self.copies.0.insert(gltf.id(), made.clone());
        Some(made)
    }

    fn textured_cover(
        &mut self,
        cover: &Handle<CoverMaterial>,
        texture: Handle<Image>,
    ) -> Handle<CoverMaterial> {
        let key = (cover.id(), texture.id());
        if let Some(textured) = self.textured.0.get(&key) {
            return textured.clone();
        }
        let Some(mut material) = self.covers.get(cover).cloned() else {
            return cover.clone();
        };
        material.base.base_color_texture = Some(texture);
        let textured = self.covers.add(material);
        self.textured.0.insert(key, textured.clone());
        textured
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
                    material: primitive.material.clone(),
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
    // Only the GPU reads a cover's mesh, which would otherwise be held twice.
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
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
