//! The geometry a ground's cover is laid out as, from the paint: a sheet over the painted cells,
//! blades standing up from the terrain, or copies of a prop strewn over it. Every vertex carries,
//! in its second uv, how far up the geometry it stands (0 at the root, 1 at the top) and how
//! heavily the material is painted there, for the shader that moves and paints it. A strewn
//! copy is an instance of its prop's own mesh, which carries only how far up the prop a vertex
//! stands; the rest of its marks and its tint ride on its tag.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, Frustum, MeshAabb};
use bevy::color::Mix;
use bevy::ecs::entity_disabling::Disabled;
use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::light::NotShadowCaster;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::mesh::{Indices, MeshTag, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::Shader;

use super::lod::GRAIN_PIXELS;
use super::merged::{self, MOST_MERGED_VERTICES, Shape};
use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rng};
use crate::core::units::{GROUND_ORIGIN, Meters};
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, VfxShaderKey};

/// A cover's material: the engine's lit surface under the content shader its ground names.
pub type CoverMaterial = ExtendedMaterial<StandardMaterial, CoverVfx>;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(VfxShaderKey)]
pub struct CoverVfx {
    #[uniform(100)]
    pub site: SiteUniform,
    #[uniform(101)]
    pub params: ParamsUniform,
    #[texture(102, sample_type = "float", filterable = false)]
    pub weather: Handle<Image>,
    #[uniform(103)]
    pub strewn: StrewnUniform,
    pub shader: Handle<Shader>,
}

/// What every strewn copy a material draws shares: which piece of its cover it is and the colours
/// its tint lies between. Off on a sheet's or blades' material, whose vertices carry their own.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct StrewnUniform {
    pub color_from: Vec4,
    pub color_to: Vec4,
    pub piece: u32,
    pub on: u32,
}

impl From<&CoverVfx> for VfxShaderKey {
    fn from(vfx: &CoverVfx) -> VfxShaderKey {
        VfxShaderKey::of(&vfx.shader)
    }
}

impl MaterialExtension for CoverVfx {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let shader = key.bind_group_data.handle();
        descriptor.vertex.shader = shader.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = shader;
        }
        Ok(())
    }
}

/// A cover's material, coloured by its vertices: blended, for a sheet or blades that write no
/// depth and take no ink, or masked, for the copies of a prop `strewn` as it spreads, which stand
/// solid like any other. Both of a face's sides are drawn and lit alike by the normal its piece
/// carries, never turned over on the far side, since foliage points its normals where its light
/// should fall rather than off its faces.
pub fn material(
    site: SiteUniform,
    params: ParamsUniform,
    shader: Handle<Shader>,
    weather: Handle<Image>,
    strewn: Option<&Spread>,
) -> CoverMaterial {
    let alpha_mode = match strewn {
        Some(_) => AlphaMode::Mask(0.5),
        None => AlphaMode::Blend,
    };
    let strewn = strewn.map_or_else(StrewnUniform::default, |spread| StrewnUniform {
        color_from: spread.color.0.to_vec4(),
        color_to: spread.color.1.to_vec4(),
        piece: spread.piece,
        on: 1,
    });
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            double_sided: false,
            cull_mode: None,
            alpha_mode,
            ..default()
        },
        extension: CoverVfx {
            site,
            params,
            weather,
            strewn,
            shader,
        },
    }
}

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

impl Spread {
    /// The spread as a view shows it at `pixels_per_meter` where every copy, `extent` across at
    /// its size, is smaller than [`GRAIN_PIXELS`]: as many copies as cover as much once each is
    /// grown to about that size.
    pub fn grained(&self, extent: f32, pixels_per_meter: f32) -> Spread {
        let across = extent * (self.size.0 + self.size.1) / 2.0 * pixels_per_meter;
        let grown = GRAIN_PIXELS / across.max(f32::EPSILON);
        Spread {
            per_square_meter: self.per_square_meter / (grown * grown),
            size: (self.size.0 * grown, self.size.1 * grown),
            ..*self
        }
    }
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

/// A chunk of a cover's piece waiting for the prop it strews to load: where every copy in the
/// chunk stands, and the material that draws them.
#[derive(Component)]
pub struct Strewing {
    pub gltf: Handle<Gltf>,
    pub places: Vec<Place>,
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

/// Every `stride`-th of `count` steps from 0, and `count` itself.
fn strided(count: u32, stride: u32) -> Vec<u32> {
    let mut steps: Vec<u32> = (0..count).step_by(stride.max(1) as usize).collect();
    steps.push(count);
    steps
}

fn seed_steps(seed: f32) -> f32 {
    (seed.clamp(0.0, 1.0) * SEEDS).floor()
}

/// A copy's own draw from its spread: its scale, and how far its colour lies between the spread's.
fn draw(spread: &Spread, rng: &mut Rng) -> (f32, f32) {
    let size = spread.size.0 + (spread.size.1 - spread.size.0) * rng.rand_float();
    (size, rng.rand_float())
}

/// A sheet over the patch's painted cells: a quad for every `stride` cells each way, each vertex
/// weighted by how heavily the cells around it are painted, lying a hair over the ground or, at a
/// `level`, flat at that height with the depth of ground beneath each vertex. Its uv runs over the
/// whole area, so a shader's pattern crosses from chunk to chunk unbroken. As meshes of a bounded
/// size, none when nothing is painted.
pub fn sheet(patch: &Patch, level: Option<f32>, color: LinearRgba, stride: u32) -> Vec<Mesh> {
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
    let (across, down) = (strided(columns, stride), strided(rows, stride));
    let mut positions = Vec::with_capacity(across.len() * down.len());
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut tips = Vec::with_capacity(positions.capacity());
    for row in down.iter().map(|row| patch.first.1 + row) {
        for column in across.iter().map(|column| patch.first.0 + column) {
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
        return Vec::new();
    }
    let (quads_across, quads_down) = (across.len() as u32 - 1, down.len() as u32 - 1);
    let across = quads_across + 1;
    let mut indices = Vec::with_capacity((quads_across * quads_down * 6) as usize);
    for row in 0..quads_down {
        for column in 0..quads_across {
            let corner = row * across + column;
            let (east, south, far) = (corner + 1, corner + across, corner + across + 1);
            indices.extend([corner, south, east, east, south, far]);
        }
    }
    let colors = vec![color.to_f32_array(); positions.len()];
    assemble(positions, normals, uvs, tips, Some(colors), indices)
}

/// A blade of the `shape` standing at each of `places` on the ground of `heights`, a triangle at
/// its place's size, turn and tint, its uv running across and up. As meshes of a bounded size, none
/// when nothing grows.
pub fn blades(
    heights: &Heightfield,
    places: &[Place],
    shape: &BladeShape,
    spread: &Spread,
) -> Vec<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(places.len() * 3);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut tips = Vec::with_capacity(positions.capacity());
    let mut colors = Vec::with_capacity(positions.capacity());
    for place in places {
        let root = place.transform.translation;
        let size = place.transform.scale.x;
        let across = place.transform.rotation * Vec3::X * (shape.width / 2.0 * size);
        positions.push((root - across).to_array());
        positions.push((root + across).to_array());
        positions.push((root + Vec3::Y * shape.height * size).to_array());
        normals.extend([ground_normal(heights, Pos::new(root.x, root.z)); 3]);
        uvs.extend([[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]);
        let color = spread.color.0.mix(&spread.color.1, place.tint);
        colors.extend([color.to_f32_array(); 3]);
        let mark = |measure| {
            Mark {
                measure,
                weight: place.weight,
                piece: spread.piece,
                seed: place.seed,
            }
            .pack()
        };
        tips.extend([mark(0.0), mark(0.0), mark(1.0)]);
    }
    if positions.is_empty() {
        return Vec::new();
    }
    let indices = (0..positions.len() as u32).collect();
    assemble(positions, normals, uvs, tips, Some(colors), indices)
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

/// The copies at `places`, each `extent` across at its size, as `pixels_per_meter` shows them: a
/// copy the view can tell apart stays as it is; of those smaller than [`GRAIN_PIXELS`], as many
/// stay as cover as much once each is grown to that size, picked by their own seeds, so a crowd
/// seen from farther keeps fewer of the same copies.
pub fn seen_at(places: &[Place], extent: f32, pixels_per_meter: f32) -> Vec<Place> {
    places
        .iter()
        .filter_map(|place| {
            let across = extent * place.transform.scale.x * pixels_per_meter;
            if across >= GRAIN_PIXELS {
                return Some(*place);
            }
            let grown = GRAIN_PIXELS / across.max(f32::EPSILON);
            (place.seed < 1.0 / (grown * grown)).then(|| {
                let mut kept = *place;
                kept.transform.scale *= grown;
                kept
            })
        })
        .collect()
}

/// The meshes a strewn copy of a prop is drawn with, one for each material the model draws its
/// `pieces` with: positions and normals carried through the piece's transform, the piece's uv and
/// colours kept (white where it has none, so a copy's tint has colours to fall on), and every
/// vertex marked how far up the whole prop it stands. Empty when there is nothing to copy.
pub fn copy_meshes(pieces: &[Piece]) -> Vec<(Option<Handle<GltfMaterial>>, Mesh)> {
    let (drawn, height) = by_material(pieces);
    drawn
        .into_iter()
        .map(|drawn| (drawn.material, merged(drawn.vertices, height)))
        .collect()
}

/// A prop's model as far chunks strew it merged: for each material it is drawn in, its pieces as
/// one shape with how far up the whole prop each vertex stands; and how far across it measures and
/// how many triangles it has in all.
pub struct CopyModel {
    shapes: Vec<CopyShape>,
    extent: f32,
    triangles: usize,
}

struct CopyShape {
    material: Option<Handle<GltfMaterial>>,
    shape: Shape,
    measures: Vec<f32>,
}

impl CopyModel {
    /// The model of `pieces` ready to merge, worked out off the main thread. A strewn copy is cut
    /// out of its texture, so its shapes thin their islands once simplifying is done.
    pub fn of(pieces: &[Piece]) -> CopyModel {
        let (drawn, height) = by_material(pieces);
        let shapes: Vec<CopyShape> = drawn
            .into_iter()
            .map(|DrawnIn { material, vertices }| {
                let measures = vertices
                    .positions
                    .iter()
                    .map(|position| (position.y / height).clamp(0.0, 1.0))
                    .collect();
                CopyShape {
                    material,
                    shape: Shape::new(vertices, true),
                    measures,
                }
            })
            .collect();
        CopyModel {
            extent: merged::extent(shapes.iter().map(|copy| &copy.shape)),
            triangles: shapes.iter().map(|copy| copy.shape.triangles()).sum(),
            shapes,
        }
    }

    pub fn extent(&self) -> f32 {
        self.extent
    }
}

/// The copies of `model` at `places`, strewn by `spread` and seen at `pixels_per_meter`, merged
/// into meshes of a bounded size for each material the model is drawn in: every copy at the level its pixels need,
/// each vertex marked as a blade's is and coloured by its copy's tint, for a material whose
/// copies carry their own marks. Empty when there is nothing to strew.
pub fn merged_copies(
    places: &[Place],
    model: &CopyModel,
    spread: &Spread,
    pixels_per_meter: f32,
) -> Vec<(Option<Handle<GltfMaterial>>, Mesh)> {
    model
        .shapes
        .iter()
        .flat_map(|copy| {
            let share = copy.shape.triangles() as f32 / model.triangles.max(1) as f32;
            let mut positions = Vec::new();
            let mut normals = Vec::new();
            let mut uvs = Vec::new();
            let mut tips = Vec::new();
            let mut colors = Vec::new();
            let mut indices = Vec::new();
            for place in places {
                let size = place.transform.scale.x;
                let covered = merged::covered_pixels(model.extent * size * pixels_per_meter);
                let level = copy.shape.level(pixels_per_meter * size, covered * share);
                let tint = spread.color.0.mix(&spread.color.1, place.tint).to_vec4();
                let matrix = place.transform.to_matrix();
                let base = positions.len() as u32;
                for (&vertex, &position) in level.vertices.iter().zip(&level.positions) {
                    let vertex = vertex as usize;
                    let vertices = &copy.shape.vertices;
                    positions.push(matrix.transform_point3(position).to_array());
                    normals.push((place.transform.rotation * vertices.normals[vertex]).to_array());
                    uvs.push(vertices.uvs[vertex]);
                    let own = vertices
                        .colors
                        .as_ref()
                        .map_or(Vec4::ONE, |colors| Vec4::from_array(colors[vertex]));
                    colors.push((own * tint).to_array());
                    tips.push(
                        Mark {
                            measure: copy.measures[vertex],
                            weight: place.weight,
                            piece: spread.piece,
                            seed: place.seed,
                        }
                        .pack(),
                    );
                }
                indices.extend(level.indices.iter().map(|index| base + index));
            }
            assemble(positions, normals, uvs, tips, Some(colors), indices)
                .into_iter()
                .map(|mesh| (copy.material.clone(), mesh))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The vertices of `pieces` joined into one piece for each material they are drawn in, and how
/// high the whole prop stands.
fn by_material(pieces: &[Piece]) -> (Vec<DrawnIn>, f32) {
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
    let drawn = materials
        .into_iter()
        .map(|material| {
            let drawn: Vec<&PieceVertices> = placed
                .iter()
                .filter(|(piece, _)| piece.material == material)
                .map(|(_, vertices)| vertices)
                .collect();
            DrawnIn {
                material,
                vertices: joined(&drawn),
            }
        })
        .collect();
    (drawn, height)
}

/// The vertices of a prop's pieces drawn in one of its materials.
struct DrawnIn {
    material: Option<Handle<GltfMaterial>>,
    vertices: PieceVertices,
}

/// The pieces' vertices as one piece's.
fn joined(pieces: &[&PieceVertices]) -> PieceVertices {
    let mut joined = PieceVertices {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        colors: Some(Vec::new()),
        indices: Vec::new(),
    };
    for piece in pieces {
        let base = joined.positions.len() as u32;
        joined.positions.extend(&piece.positions);
        joined.normals.extend(&piece.normals);
        joined.uvs.extend(&piece.uvs);
        if let Some(colors) = joined.colors.as_mut() {
            match &piece.colors {
                Some(piece_colors) => colors.extend(piece_colors),
                None => colors.extend(std::iter::repeat_n([1.0; 4], piece.positions.len())),
            }
        }
        joined
            .indices
            .extend(piece.indices.iter().map(|index| base + index));
    }
    joined
}

/// The pieces of a strewn prop's model, once it has loaded.
pub fn model_pieces(world: &World, gltf: &Handle<Gltf>) -> Option<Vec<Piece>> {
    pieces_of(
        world.resource::<Assets<Gltf>>().get(gltf)?,
        world.resource::<Assets<GltfNode>>(),
        world.resource::<Assets<GltfMesh>>(),
        world.resource::<Assets<Mesh>>(),
    )
}

/// The material a strewn copy's merged meshes are drawn in: `strewn`'s, its copies carrying their
/// own marks and colours, in the texture the model's `material` draws with.
pub fn merged_material(
    world: &mut World,
    strewn: &Handle<CoverMaterial>,
    material: Option<&Handle<GltfMaterial>>,
) -> Option<Handle<CoverMaterial>> {
    let texture = material.and_then(|material| {
        world
            .resource::<Assets<GltfMaterial>>()
            .get(material)?
            .base_color_texture
            .clone()
    });
    let key = MergedCoverKey {
        strewn: strewn.id(),
        texture: texture.as_ref().map(Handle::id),
    };
    if let Some(made) = world.resource::<MergedCovers>().0.get(&key) {
        return Some(made.clone());
    }
    let mut merged = world
        .resource::<Assets<CoverMaterial>>()
        .get(strewn)?
        .clone();
    merged.extension.strewn.on = 0;
    if texture.is_some() {
        merged.base.base_color_texture = texture;
    }
    let made = world.resource_mut::<Assets<CoverMaterial>>().add(merged);
    world
        .resource_mut::<MergedCovers>()
        .0
        .insert(key, made.clone());
    Some(made)
}

/// The materials made for strewn copies' merged meshes, by the strewn material and the texture
/// they are drawn in.
#[derive(Resource, Default)]
pub(super) struct MergedCovers(HashMap<MergedCoverKey, Handle<CoverMaterial>>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MergedCoverKey {
    strewn: AssetId<CoverMaterial>,
    texture: Option<AssetId<Image>>,
}

/// The piece as a mesh, each vertex marked how far up `height` it stands.
fn merged(piece: PieceVertices, height: f32) -> Mesh {
    let tips = piece
        .positions
        .iter()
        .map(|position| {
            Mark {
                measure: (position.y / height).clamp(0.0, 1.0),
                weight: 0.0,
                piece: 0,
                seed: 0.0,
            }
            .pack()
        })
        .collect();
    mesh_of(
        piece
            .positions
            .iter()
            .map(|position| position.to_array())
            .collect(),
        piece
            .normals
            .iter()
            .map(|normal| normal.to_array())
            .collect(),
        piece.uvs,
        tips,
        piece.colors,
        piece.indices,
    )
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
    mut merged: ResMut<MergedCovers>,
    mut copies: ResMut<CopyMeshes>,
) {
    let gone = super::dropped(&mut covers);
    if !gone.is_empty() {
        textured.0.retain(|(cover, _), _| !gone.contains(cover));
        merged.0.retain(|key, _| !gone.contains(&key.strewn));
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

/// A chunk whose prop has loaded gets, for each of the prop's meshes, a chunk of copies to strew
/// at every one of its places once a camera first sees it: an instance of the mesh drawn with the
/// piece's material in that mesh's texture, so a copy keeps the shapes its texture cuts out.
pub(super) fn strew_loaded(
    mut assets: StrewnAssets,
    waiting: Query<(Entity, &Strewing)>,
    mut commands: Commands,
) {
    for (entity, strewing) in &waiting {
        let Some(meshes) = assets.copy_meshes(&strewing.gltf) else {
            continue;
        };
        let parts: Vec<_> = meshes
            .into_iter()
            .map(
                |CopyMesh {
                     material,
                     mesh,
                     bounds,
                 }| {
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
                    (
                        StrewnChunk {
                            bounds: enclosing(&strewing.places, &bounds),
                            copies: strewing.places.len(),
                            unstrewn: Some(Unstrewn {
                                places: strewing.places.clone(),
                                mesh,
                                material: drawn,
                                bounds,
                            }),
                        },
                        Transform::default(),
                        Visibility::default(),
                    )
                },
            )
            .collect();
        // The scenery may take the chunk down within the frame it is strewn in, which leaves
        // nothing to strew.
        commands
            .entity(entity)
            .queue_silenced(move |mut chunk: EntityWorldMut| {
                chunk.remove::<Strewing>().with_children(|strewn| {
                    for part in parts {
                        strewn.spawn(part);
                    }
                });
            });
    }
}

/// A chunk of a piece's strewn copies of one of its prop's meshes, culled as one by
/// [`cull_strewn_chunks`]: a view sees only a few of the chunks standing around it, and the copies
/// of the rest are too many to test one by one each frame. Its copies are strewn only once a
/// camera first sees it, so ground nobody looks at costs nothing however thickly it is strewn.
#[derive(Component)]
pub(super) struct StrewnChunk {
    bounds: Aabb,
    copies: usize,
    unstrewn: Option<Unstrewn>,
}

/// The copies a chunk strews when it is first seen: a copy of `mesh` within `bounds` at every
/// place, drawn with `material`.
struct Unstrewn {
    places: Vec<Place>,
    mesh: Handle<Mesh>,
    material: Handle<CoverMaterial>,
    bounds: Aabb,
}

/// Disables every chunk no camera sees, copies and all, and enables every chunk one comes to see,
/// strewing its copies the first time, as many copies a frame as keep the frame short. A hidden entity still costs every frame's visibility and
/// extraction a look, and an area holds far more copies than a view takes in, so the unseen leave
/// every query instead. Propagation passes a disabled entity over, so this runs once the frame's
/// propagation is done, and a copy keeps where it stood and whether it showed as it was disabled:
/// a cover must not move or hide while it stands.
pub(super) fn cull_strewn_chunks(
    cameras: Query<(&Camera, &Frustum), With<Camera3d>>,
    mut chunks: Query<(Entity, &mut StrewnChunk, &GlobalTransform, Has<Disabled>)>,
    mut commands: Commands,
) {
    let mut strewable = COPIES_STREWN_PER_FRAME;
    let mut switchable = COPIES_SWITCHED_PER_FRAME;
    for (entity, mut chunk, placed, disabled) in &mut chunks {
        let seen = cameras.iter().all(|(camera, _)| !camera.is_active)
            || cameras.iter().any(|(camera, frustum)| {
                camera.is_active
                    && frustum.intersects_obb(&chunk.bounds, &placed.affine(), true, false)
            });
        if seen != !disabled && switchable > 0 {
            switchable = switchable.saturating_sub(chunk.copies);
            if seen {
                commands
                    .entity(entity)
                    .remove_recursive::<Children, Disabled>();
            } else {
                commands
                    .entity(entity)
                    .insert_recursive::<Children>(Disabled);
            }
        }
        if seen
            && strewable > 0
            && let Some(unstrewn) = chunk.unstrewn.take()
        {
            strewable = strewable.saturating_sub(unstrewn.places.len());
            let copies: Vec<_> = unstrewn
                .places
                .iter()
                .map(|place| {
                    (
                        Mesh3d(unstrewn.mesh.clone()),
                        MeshMaterial3d(unstrewn.material.clone()),
                        place.transform,
                        place.tag(),
                        unstrewn.bounds,
                        NotShadowCaster,
                        ChildOf(entity),
                    )
                })
                .collect();
            commands.spawn_batch(copies);
        }
    }
}

/// Whether `chunk`, spawned with a [`Strewing`], still has copies to strew where a camera sees
/// them: its prop yet to load, or a chunk of its copies in sight but not yet strewn.
pub fn strewing(world: &World, chunk: Entity) -> bool {
    let Ok(chunk) = world.get_entity(chunk) else {
        return false;
    };
    chunk.contains::<Strewing>()
        || chunk.get::<Children>().is_some_and(|parts| {
            parts.iter().any(|part| {
                world.get_entity(part).is_ok_and(|part| {
                    !part.contains::<Disabled>()
                        && part
                            .get::<StrewnChunk>()
                            .is_some_and(|strewn| strewn.unstrewn.is_some())
                })
            })
        })
}

const COPIES_STREWN_PER_FRAME: usize = 2000;
const COPIES_SWITCHED_PER_FRAME: usize = 4000;

/// The box around every copy at `places` of a mesh within `bounds`.
fn enclosing(places: &[Place], bounds: &Aabb) -> Aabb {
    let (low, high) = places
        .iter()
        .flat_map(|place| {
            let matrix = place.transform.to_matrix();
            corners(bounds).map(move |corner| matrix.transform_point3(corner))
        })
        .fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(low, high), point| (low.min(point), high.max(point)),
        );
    Aabb::from_min_max(low, high)
}

fn corners(bounds: &Aabb) -> impl Iterator<Item = Vec3> {
    let (center, half) = (Vec3::from(bounds.center), Vec3::from(bounds.half_extents));
    (0..8).map(move |corner| {
        let side = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
        center + half * Vec3::new(side(1), side(2), side(4))
    })
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
pub(super) struct PieceVertices {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
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
pub(super) fn pieces_of(
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

pub(super) fn piece_vertices(piece: &Piece) -> Option<PieceVertices> {
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

/// The meshes of `indices`' triangles over the vertices, each holding at most
/// [`MOST_MERGED_VERTICES`], so a large cover goes to the GPU as several small meshes, a few a
/// frame, rather than one large one.
fn assemble(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    tips: Vec<[f32; 2]>,
    colors: Option<Vec<[f32; 4]>>,
    indices: Vec<u32>,
) -> Vec<Mesh> {
    if positions.len() <= MOST_MERGED_VERTICES {
        return vec![mesh_of(positions, normals, uvs, tips, colors, indices)];
    }
    let mut meshes = Vec::new();
    let mut taken = vec![None; positions.len()];
    let mut part = Part::default();
    for triangle in indices.as_chunks::<3>().0 {
        let fresh = triangle
            .iter()
            .filter(|&&vertex| taken[vertex as usize].is_none())
            .count();
        if part.sources.len() + fresh > MOST_MERGED_VERTICES {
            for &source in &part.sources {
                taken[source as usize] = None;
            }
            meshes.push(std::mem::take(&mut part).mesh(&positions, &normals, &uvs, &tips, &colors));
        }
        for &vertex in triangle {
            let index = *taken[vertex as usize].get_or_insert_with(|| {
                part.sources.push(vertex);
                part.sources.len() as u32 - 1
            });
            part.indices.push(index);
        }
    }
    if !part.indices.is_empty() {
        meshes.push(part.mesh(&positions, &normals, &uvs, &tips, &colors));
    }
    meshes
}

/// A mesh being cut out of a larger one: the vertices it takes, by their index there, and its
/// triangles over them.
#[derive(Default)]
struct Part {
    sources: Vec<u32>,
    indices: Vec<u32>,
}

impl Part {
    fn mesh(
        self,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: &[[f32; 2]],
        tips: &[[f32; 2]],
        colors: &Option<Vec<[f32; 4]>>,
    ) -> Mesh {
        mesh_of(
            picked(&self.sources, positions),
            picked(&self.sources, normals),
            picked(&self.sources, uvs),
            picked(&self.sources, tips),
            colors
                .as_deref()
                .map(|colors| picked(&self.sources, colors)),
            self.indices,
        )
    }
}

fn picked<T: Copy>(sources: &[u32], values: &[T]) -> Vec<T> {
    sources
        .iter()
        .map(|&source| values[source as usize])
        .collect()
}

fn mesh_of(
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
    mesh.compress_indices();
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
