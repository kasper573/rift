//! Many props of the world drawn as one mesh for each material they are drawn in, for where they
//! stand too far off the eye to be worth an entity each. Every vertex of a merged mesh stands where
//! its prop puts it in the world, and carries where the piece of the model it belongs to stands,
//! how much it is grown and how it is turned, so the game's shader draws it just as it draws the
//! prop on its own.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfAssetLabel, GltfMaterial, GltfMesh, GltfNode};
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology, VertexFormat};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::cover::{self, Piece, PieceVertices};
use super::lod::{self, GRAIN_PIXELS, STRAY_PIXELS};
use super::material::{MaterialLooks, Wardrobe, WorldMaterial};

/// Where the piece a merged vertex belongs to stands in the world, and in `w` how much it is grown.
pub const ATTRIBUTE_PIECE_ORIGIN: MeshVertexAttribute =
    MeshVertexAttribute::new("PieceOrigin", 0x7269_6674_6d01, VertexFormat::Float32x4);

/// How the piece a merged vertex belongs to is turned in the world, as a quaternion.
pub const ATTRIBUTE_PIECE_TURN: MeshVertexAttribute =
    MeshVertexAttribute::new("PieceTurn", 0x7269_6674_6d02, VertexFormat::Float32x4);

/// A prop's model ready to merge: each of its pieces with its shape in the model and the world
/// material it is drawn in; and how far across the model measures, and how many triangles it has
/// in all.
pub struct MergedModel {
    pieces: Vec<MergedPiece>,
    extent: f32,
    triangles: usize,
}

/// Geometry ready to merge at the level a view needs: its vertices, and the triangles it is drawn
/// with at every level it comes down to, finest first, its own at the head.
pub struct Shape {
    pub(super) vertices: PieceVertices,
    pub(super) levels: Vec<PieceLevel>,
    /// How many of the levels the simplifier made, the shape's own first; any after them thin its
    /// islands.
    simplified: usize,
}

impl Shape {
    /// The shape of `vertices` at its own triangles, then coarser and coarser: what the simplifier
    /// brings them to, each with how far its shape strays, and where its `silhouette` is cut
    /// out of its texture, its islands thinned on from there once they are too small to tell
    /// apart.
    pub(super) fn new(vertices: PieceVertices, silhouette: Silhouette) -> Shape {
        let positions: Vec<[f32; 3]> = vertices
            .positions
            .iter()
            .map(|position| position.to_array())
            .collect();
        let normals: Vec<[f32; 3]> = vertices
            .normals
            .iter()
            .map(|normal| normal.to_array())
            .collect();
        let mut levels: Vec<PieceLevel> =
            std::iter::once(PieceLevel::of(&vertices, &vertices.indices, f32::INFINITY))
                .chain(
                    lod::simplify(
                        &vertices.indices,
                        &positions,
                        lod::Surface {
                            uvs: Some(&vertices.uvs),
                            normals: Some(&normals),
                        },
                    )
                    .iter()
                    .map(|(kept, strays)| PieceLevel::of(&vertices, kept, STRAY_PIXELS / strays)),
                )
                .collect();
        let simplified = levels.len();
        if silhouette == Silhouette::CutOut
            && let Some(coarsest) = levels.last()
        {
            let thinned = thinned(coarsest);
            levels.extend(thinned);
        }
        Shape {
            vertices,
            levels,
            simplified,
        }
    }

    pub(super) fn triangles(&self) -> usize {
        self.levels[0].triangles()
    }

    /// The level to draw the shape at, seen at `pixels_per_unit` over `covered` pixels: the
    /// coarsest that looks as the shape does there, and of those the simplifier made, coarser
    /// still while it takes more triangles than the pixels it covers, as no picture shows more of
    /// a shape than a triangle a pixel. Thinned islands stand in only where they look as the
    /// shape does.
    pub(super) fn level(&self, pixels_per_unit: f32, covered: f32) -> &PieceLevel {
        let by_look = self
            .levels
            .iter()
            .rposition(|level| pixels_per_unit <= level.shown_up_to)
            .unwrap_or(0);
        let simplified = &self.levels[..self.simplified];
        let by_pixels = simplified
            .iter()
            .position(|level| level.triangles() as f32 <= covered)
            .unwrap_or(simplified.len() - 1);
        &self.levels[by_look.max(by_pixels)]
    }
}

/// What a model is merged from, once it and its materials have loaded.
pub struct ModelParts(Vec<ModelPart>);

impl ModelParts {
    /// The model ready to merge: its pieces' shapes, worked out off the main thread.
    pub fn prepare(self) -> MergedModel {
        let pieces: Vec<MergedPiece> = self
            .0
            .into_iter()
            .filter_map(
                |ModelPart {
                     piece,
                     material,
                     silhouette,
                 }| {
                    let mut vertices = cover::piece_vertices(&piece)?;
                    let scale = piece.transform.scale;
                    if scale.x * scale.y * scale.z < 0.0 {
                        for triangle in vertices.indices.as_chunks_mut::<3>().0 {
                            triangle.swap(1, 2);
                        }
                    }
                    Some(MergedPiece {
                        material,
                        transform: piece.transform,
                        shape: Shape::new(vertices, silhouette),
                    })
                },
            )
            .collect();
        MergedModel {
            extent: extent(pieces.iter().map(|piece| &piece.shape)),
            triangles: pieces.iter().map(|piece| piece.shape.triangles()).sum(),
            pieces,
        }
    }
}

/// A prop standing among those merged: its model, where it stands, and whether it casts a shadow.
pub struct MergedPlace {
    pub model: Arc<MergedModel>,
    pub transform: Transform,
    pub shadow: bool,
}

/// A merged mesh: what it is drawn in, whether it casts a shadow, and its bounds.
pub struct MergedMesh {
    pub material: Handle<WorldMaterial>,
    pub shadow: bool,
    pub mesh: Mesh,
    pub bounds: Aabb,
}

/// The assets a prop's model is merged from.
#[derive(SystemParam)]
pub struct ModelSources<'w> {
    server: Res<'w, AssetServer>,
    gltfs: Res<'w, Assets<Gltf>>,
    nodes: Res<'w, Assets<GltfNode>>,
    gltf_meshes: Res<'w, Assets<GltfMesh>>,
    meshes: Res<'w, Assets<Mesh>>,
    wardrobe: Wardrobe<'w>,
}

impl ModelSources<'_> {
    /// What `gltf` is merged from, its materials dressed by `looks` as the scene's would be;
    /// none until it has loaded.
    pub fn parts(&mut self, gltf: &Handle<Gltf>, looks: &MaterialLooks) -> Option<ModelParts> {
        let model = self.gltfs.get(gltf)?;
        let names: HashMap<AssetId<GltfMaterial>, String> = model
            .named_materials
            .iter()
            .map(|(name, material)| (material.id(), name.to_string()))
            .collect();
        let pieces = cover::pieces_of(model, &self.nodes, &self.gltf_meshes, &self.meshes)?;
        pieces
            .into_iter()
            .map(|piece| {
                let standard = self.standard_of(gltf, piece.material.as_ref())?;
                let look = piece
                    .material
                    .as_ref()
                    .and_then(|material| names.get(&material.id()))
                    .and_then(|name| looks.0.iter().find(|look| look.material == *name));
                let material = self.wardrobe.material_for(&standard, look)?;
                let silhouette = if self.wardrobe.cuts_out(&material) {
                    Silhouette::CutOut
                } else {
                    Silhouette::Solid
                };
                Some(ModelPart {
                    piece,
                    material,
                    silhouette,
                })
            })
            .collect::<Option<Vec<_>>>()
            .map(ModelParts)
    }

    /// The standard material the glTF's loader made for `material`, or for the model's default
    /// one where a piece names none.
    fn standard_of(
        &self,
        gltf: &Handle<Gltf>,
        material: Option<&Handle<GltfMaterial>>,
    ) -> Option<Handle<StandardMaterial>> {
        let path = match material {
            Some(material) => self.server.get_path(material.id())?.into_owned(),
            None => self
                .server
                .get_path(gltf.id())?
                .into_owned()
                .with_label(GltfAssetLabel::DefaultMaterial.to_string()),
        };
        let label = format!("{}/std", path.label()?);
        self.server.get_handle(path.with_label(label))
    }
}

/// The props at `places` merged, meshes of at most [`MOST_MERGED_VERTICES`] for each material they
/// are drawn in and each way they cast a shadow, seen at `pixels_per_meter`, every piece at the
/// level [`Shape::level`] picks for its share of what its prop covers.
pub fn merge(places: &[MergedPlace], pixels_per_meter: f32) -> Vec<MergedMesh> {
    let mut merging: HashMap<(AssetId<WorldMaterial>, bool), Vec<Merging>> = HashMap::default();
    for place in places {
        let model = &place.model;
        let scale = place.transform.scale.x.abs();
        let covered = covered_pixels(model.extent * scale * pixels_per_meter);
        for piece in &model.pieces {
            let placed = place.transform * piece.transform;
            let share = piece.shape.triangles() as f32 / model.triangles.max(1) as f32;
            let level = piece.shape.level(pixels_per_meter * scale, covered * share);
            let meshes = merging
                .entry((piece.material.id(), place.shadow))
                .or_default();
            if meshes.last().is_none_or(|last| {
                last.positions.len() + level.vertices.len() > MOST_MERGED_VERTICES
            }) {
                meshes.push(Merging::new(piece.material.clone(), place.shadow));
            }
            if let Some(last) = meshes.last_mut() {
                last.add(&piece.shape.vertices, level, place.transform, placed);
            }
        }
    }
    merging
        .into_values()
        .flatten()
        .map(Merging::finish)
        .collect()
}

/// The most vertices a merged mesh holds: as many as two-byte indices reach, so a region's props
/// go to the GPU as several small meshes, a few a frame, rather than one large one.
pub const MOST_MERGED_VERTICES: usize = u16::MAX as usize;

/// A shape's triangles at a level: the vertices of the shape they use, where each lies, and the
/// triangles over them, with the most pixels a unit of the shape may span for the level to look
/// as the shape does.
pub(super) struct PieceLevel {
    shown_up_to: f32,
    pub(super) vertices: Vec<u32>,
    pub(super) positions: Vec<Vec3>,
    pub(super) indices: Vec<u32>,
}

impl PieceLevel {
    fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The level drawing the triangles `kept` index over the vertices, looking as the shape does
    /// up to `shown_up_to` pixels a unit.
    fn of(vertices: &PieceVertices, kept: &[u32], shown_up_to: f32) -> PieceLevel {
        PieceLevel::drawing(kept.iter().copied(), shown_up_to, |vertex| {
            (vertex, vertices.positions[vertex as usize])
        })
    }

    /// The level drawing the triangles `kept` index, each vertex standing for the piece's vertex
    /// and at the place `placed` gives it.
    fn drawing(
        kept: impl IntoIterator<Item = u32>,
        shown_up_to: f32,
        placed: impl Fn(u32) -> (u32, Vec3),
    ) -> PieceLevel {
        let mut level = PieceLevel {
            shown_up_to,
            vertices: Vec::new(),
            positions: Vec::new(),
            indices: Vec::new(),
        };
        let mut used: HashMap<u32, u32> = HashMap::default();
        for vertex in kept {
            let index = *used.entry(vertex).or_insert_with(|| {
                let (stands_for, at) = placed(vertex);
                level.vertices.push(stands_for);
                level.positions.push(at);
                level.vertices.len() as u32 - 1
            });
            level.indices.push(index);
        }
        level
    }
}

/// Whether a piece shows its triangles whole, or as its material cuts it out of its texture, as
/// foliage is, whose islands may then thin once too small to tell apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Silhouette {
    Solid,
    CutOut,
}

/// How many pixels a shape `across` pixels wide covers, taken round.
pub(super) fn covered_pixels(across: f32) -> f32 {
    std::f32::consts::FRAC_PI_4 * across * across
}

/// How far across `shapes` measure together.
pub(super) fn extent<'a>(shapes: impl Iterator<Item = &'a Shape>) -> f32 {
    let (low, high) = shapes
        .flat_map(|shape| shape.vertices.positions.iter())
        .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(low, high), &at| {
            (low.min(at), high.max(at))
        });
    (high - low).max_element().max(0.0)
}

/// A piece of a model, the world material it is drawn in, and the silhouette that material gives
/// it.
struct ModelPart {
    piece: Piece,
    material: Handle<WorldMaterial>,
    silhouette: Silhouette,
}

struct MergedPiece {
    material: Handle<WorldMaterial>,
    transform: Transform,
    shape: Shape,
}

/// A level's islands, triangles joined by the vertices they share, thinned to every second one,
/// then every fourth, while more than one is left: each island kept grown about its middle to make
/// up the area of those dropped, so what the islands cover stays as thick. A thinner level looks
/// as the shape does only while its grown islands span no more than [`GRAIN_PIXELS`], too few for
/// the view to tell them apart from the islands they stand in for.
fn thinned(level: &PieceLevel) -> Vec<PieceLevel> {
    let triangles = level.indices.as_chunks::<3>().0;
    let mut joined: Vec<u32> = (0..level.vertices.len() as u32).collect();
    let root = |joined: &mut Vec<u32>, mut at: u32| {
        while joined[at as usize] != at {
            joined[at as usize] = joined[joined[at as usize] as usize];
            at = joined[at as usize];
        }
        at
    };
    for triangle in triangles {
        let first = root(&mut joined, triangle[0]);
        for &vertex in &triangle[1..] {
            let other = root(&mut joined, vertex);
            joined[other as usize] = first;
        }
    }
    let mut islands: HashMap<u32, usize> = HashMap::default();
    let island_of_vertex: Vec<usize> = (0..level.vertices.len() as u32)
        .map(|vertex| {
            let found = root(&mut joined, vertex);
            let count = islands.len();
            *islands.entry(found).or_insert(count)
        })
        .collect();
    let mut middles = vec![(Vec3::ZERO, 0.0f32); islands.len()];
    let mut bounds = vec![(Vec3::INFINITY, Vec3::NEG_INFINITY); islands.len()];
    for (vertex, &island) in island_of_vertex.iter().enumerate() {
        let at = level.positions[vertex];
        middles[island].0 += at;
        middles[island].1 += 1.0;
        bounds[island] = (bounds[island].0.min(at), bounds[island].1.max(at));
    }
    let widest = bounds
        .iter()
        .map(|(low, high)| (*high - *low).max_element())
        .fold(0.0f32, f32::max);
    let mut levels = Vec::new();
    let mut every = 2;
    while islands.len() / every >= 1 {
        let grown = (every as f32).sqrt();
        let kept = triangles
            .iter()
            .filter(|triangle| island_of_vertex[triangle[0] as usize].is_multiple_of(every))
            .flat_map(|triangle| triangle.iter().copied());
        let shown_up_to = level
            .shown_up_to
            .min(GRAIN_PIXELS / (widest * grown).max(f32::EPSILON));
        levels.push(PieceLevel::drawing(kept, shown_up_to, |vertex| {
            let (sum, count) = middles[island_of_vertex[vertex as usize]];
            let middle = sum / count;
            let at = middle + (level.positions[vertex as usize] - middle) * grown;
            (level.vertices[vertex as usize], at)
        }));
        every *= 2;
    }
    levels
}

/// A merged mesh as it is filled.
struct Merging {
    material: Handle<WorldMaterial>,
    shadow: bool,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    origins: Vec<[f32; 4]>,
    turns: Vec<[f32; 4]>,
    indices: Vec<u32>,
    low: Vec3,
    high: Vec3,
}

impl Merging {
    fn new(material: Handle<WorldMaterial>, shadow: bool) -> Merging {
        Merging {
            material,
            shadow,
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
            origins: Vec::new(),
            turns: Vec::new(),
            indices: Vec::new(),
            low: Vec3::INFINITY,
            high: Vec3::NEG_INFINITY,
        }
    }

    /// A piece at `level`, its vertices carried from the model to where `prop` stands, each
    /// marked with how its piece stands as `placed`.
    fn add(
        &mut self,
        vertices: &PieceVertices,
        level: &PieceLevel,
        prop: Transform,
        placed: Transform,
    ) {
        let base = self.positions.len() as u32;
        let matrix = prop.to_matrix();
        let origin = placed.translation.extend(placed.scale.x.abs()).to_array();
        let turn = placed.rotation.to_array();
        for (&vertex, &position) in level.vertices.iter().zip(&level.positions) {
            let vertex = vertex as usize;
            let position = matrix.transform_point3(position);
            self.low = self.low.min(position);
            self.high = self.high.max(position);
            self.positions.push(position.to_array());
            self.normals
                .push((prop.rotation * vertices.normals[vertex]).to_array());
            self.uvs.push(vertices.uvs[vertex]);
            self.colors.push(
                vertices
                    .colors
                    .as_ref()
                    .map_or([1.0; 4], |colors| colors[vertex]),
            );
            self.origins.push(origin);
            self.turns.push(turn);
        }
        self.indices
            .extend(level.indices.iter().map(|index| base + index));
    }

    fn finish(self) -> MergedMesh {
        // Only the GPU reads a merged mesh, which would otherwise be held twice.
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_attribute(ATTRIBUTE_PIECE_ORIGIN, self.origins)
        .with_inserted_attribute(ATTRIBUTE_PIECE_TURN, self.turns)
        .with_inserted_indices(Indices::U32(self.indices));
        mesh.compress_indices();
        MergedMesh {
            material: self.material,
            shadow: self.shadow,
            mesh,
            bounds: Aabb::from_min_max(self.low, self.high),
        }
    }
}
