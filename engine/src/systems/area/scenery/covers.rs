//! The covers an area's terrains grow, over the regions the scenery is cut into for the view. In
//! a leaf a cover stands chunk by chunk: its sheet, its blades and the copies of the props it
//! strews, each copy an instance of its prop. In every coarser region it stands merged, a mesh for
//! each of its pieces, as thin as the region is far: its sheet coarser, its blades and copies
//! fewer and grown to cover as much, and no more triangles than the pixels the region covers can
//! show, so a view over a whole area stands a few meshes, however thickly it is grown.

use std::sync::Arc;

use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::gltf::{Gltf, GltfMaterial};
use bevy::light::NotShadowCaster;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::regions::{Cut, Merge, Region, Unstood};
use super::{
    Budget, KEEP_SHARE, Making, PLACEMENT_CELL, Seen, Streaming, distance_to, mesh_bytes, take_away,
};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rect, Size};
use crate::core::render::camera::ground_pos;
use crate::core::render::world::cover::{self, CopyModel, CoverMaterial};
use crate::core::units::{MeterSize, Meters, Radians, Seconds};
use crate::core::vfx::ParamFill;
use crate::core::vfx::render::{self as vfx_render, Run, SiteFrame};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::uniform::{ParamsUniform, WeatherImage};
use crate::systems::area::Area;
use crate::systems::area::prop::PropId;
use crate::systems::area::terrain::{CoverPiece, PieceShape};

/// How many pixels of a far region a merged cover spends a triangle on: the region is seen whole,
/// and its cover reads as the ground's texture more than as things standing on it.
const PIXELS_PER_TRIANGLE: f32 = 16.0;

/// The covers of an area's terrains as its scenery stands them: each terrain's that has one,
/// what stands of them chunk by chunk, and what stands of them merged.
pub(super) struct Covers {
    layers: Vec<CoverLayer>,
    chunks: HashMap<ChunkKey, Option<Entity>>,
    merged: HashMap<Merge, Merged>,
    /// What stood for ground since sculpted or painted anew, standing until what stands for the
    /// ground as it is now does.
    stale: Vec<Entity>,
}

/// A chunk of a cover's piece: the layer, the piece and the chunk among the paint's.
type ChunkKey = (usize, usize, (u32, u32));

/// A cover a terrain grows, ready to stand: its slot among the area's terrains, its pieces, the
/// material its sheets and blades are drawn with, and the cover's seed.
struct CoverLayer {
    slot: usize,
    seed: u64,
    sheet_level: Option<f32>,
    ground_color: LinearRgba,
    pieces: Vec<LayerPiece>,
    blended: Handle<CoverMaterial>,
}

/// A piece of a cover as its layer stands it: its shape (none for the sheet) and how it is
/// spread, the most a copy of it measures across where that is known, and for a prop the model it
/// strews and the material its copies are drawn with.
struct LayerPiece {
    shape: Option<CoverPiece>,
    extent: Option<Meters>,
    spread: cover::Spread,
    strewn: Option<Strewn>,
}

/// A prop a piece strews: its model, the material its copies are drawn with, and its model as
/// merged regions strew it.
struct Strewn {
    gltf: Handle<Gltf>,
    material: Handle<CoverMaterial>,
    model: CopyState,
}

/// A strewn prop's model as merged regions need it: loading, being made ready to merge, or ready.
enum CopyState {
    Loading,
    Preparing(Task<CopyModel>),
    Ready(Arc<CopyModel>),
}

/// What stands of a merged region: waiting on the models of the props its covers strew, its
/// meshes being made, or standing, those made rising a few a frame.
enum Merged {
    Waiting,
    Merging(Making<Vec<FarMesh>>),
    Standing {
        stood: Vec<Entity>,
        rising: Vec<FarMesh>,
    },
}

impl Merged {
    fn into_made(self) -> Option<Vec<FarMesh>> {
        match self {
            Merged::Merging(making) => making.into_made(),
            _ => None,
        }
    }
}

/// A merged mesh of a cover's piece over a region, and for a strewn prop the material of its
/// model it is drawn in.
struct FarMesh {
    layer: usize,
    piece: usize,
    material: Option<Handle<GltfMaterial>>,
    mesh: Mesh,
    bounds: Aabb,
}

impl Covers {
    pub(super) fn new(world: &mut World, content: &Content, area: &Area) -> Covers {
        Covers {
            layers: cover_layers(world, content, area),
            chunks: HashMap::default(),
            merged: HashMap::default(),
            stale: Vec::new(),
        }
    }

    /// Stands everything again over ground sculpted or painted anew.
    pub(super) fn ground_changed(&mut self) {
        self.stale
            .extend(self.chunks.drain().filter_map(|(_, chunk)| chunk));
        for (_, merged) in self.merged.drain() {
            if let Merged::Standing { stood, .. } = merged {
                self.stale.extend(stood);
            }
        }
    }
}

/// Stands what `cut` takes in of the covers: the chunks of the leaves, and the merged meshes of
/// every coarser region; where it doesn't stand yet.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    seen: Seen,
    cut: &Cut,
    budget: &mut Budget,
) -> Unstood {
    let mut unstood = Unstood::default();
    if streaming.covers.layers.is_empty() {
        return unstood;
    }
    let missing: Vec<ChunkKey> = near_chunks(streaming, seen, cut, 1.0)
        .filter(|key| !streaming.covers.chunks.contains_key(key))
        .collect();
    let mut unstood_chunks: HashSet<(u32, u32)> = HashSet::default();
    for key in missing {
        if !unstood_chunks.is_empty() || !budget.take(|budget| &mut budget.covers) {
            unstood_chunks.insert(key.2);
            continue;
        }
        let entity = spawn_cover_chunk(world, streaming, key.0, key.1, key.2, root);
        streaming.covers.chunks.insert(key, entity);
    }
    for chunk in unstood_chunks {
        unstood.add(chunk_bounds(chunk));
    }
    let shown = shown_pieces(streaming, seen, cut);
    prepare_models(world, streaming, &shown);
    for (&merge, pieces) in &shown {
        if !stand_merge(world, streaming, root, seen, merge, pieces, budget) {
            unstood.add(streaming.regions.bounds(merge.region));
        }
    }
    unstood
}

/// Takes down what stood of the covers for an earlier view and stands for none in `cut`, wherever
/// what stands for `cut` does, and what stood for ground sculpted or painted anew once all of it
/// does.
pub(super) fn clear(
    world: &mut World,
    streaming: &mut Streaming,
    seen: Seen,
    cut: &Cut,
    unstood: &Unstood,
) {
    let kept: HashSet<ChunkKey> = near_chunks(streaming, seen, cut, KEEP_SHARE).collect();
    let covers = &mut streaming.covers;
    if unstood.is_empty() {
        for stale in covers.stale.drain(..) {
            take_away(world, stale);
        }
    }
    covers.chunks.retain(|key, chunk| {
        let keep = kept.contains(key) || !unstood.stands_over(chunk_bounds(key.2));
        if !keep && let Some(chunk) = chunk {
            take_away(world, *chunk);
        }
        keep
    });
    let wanted: HashSet<Merge> = cut.merges.iter().copied().collect();
    let regions = &streaming.regions;
    covers.merged.retain(|merge, merged| {
        let keep = wanted.contains(merge) || !unstood.stands_over(regions.bounds(merge.region));
        if !keep && let Merged::Standing { stood, .. } = merged {
            for &mesh in stood.iter() {
                take_away(world, mesh);
            }
        }
        keep
    });
}

/// Every chunk of every piece in the leaves of `cut` the view takes in, or would within `share`
/// times its reach and its pixels' distance.
fn near_chunks<'a>(
    streaming: &'a Streaming,
    seen: Seen,
    cut: &'a Cut,
    share: f32,
) -> impl Iterator<Item = ChunkKey> + 'a {
    let (columns, rows) = streaming.paint.pixels();
    let per_leaf = (PLACEMENT_CELL.0 * SAMPLES_PER_METER as f32) as u32 / cover::CHUNK_CELLS;
    let (chunk_columns, chunk_rows) = (
        columns.div_ceil(cover::CHUNK_CELLS),
        rows.div_ceil(cover::CHUNK_CELLS),
    );
    let eye = seen.view.eye();
    let covers = &streaming.covers;
    cut.leaves.iter().flat_map(move |leaf| {
        let chunks = (leaf.row * per_leaf..((leaf.row + 1) * per_leaf).min(chunk_rows)).flat_map(
            move |row| {
                (leaf.column * per_leaf..((leaf.column + 1) * per_leaf).min(chunk_columns))
                    .map(move |column| (column, row))
            },
        );
        chunks.flat_map(move |chunk| {
            let bounds = chunk_bounds(chunk);
            let in_reach = distance_to(bounds, seen.at) <= seen.reach.0 * share;
            let across = distance_to(bounds, ground_pos(eye));
            let below = eye.y - streaming.heights.height_at(bounds.center()).0;
            let from_eye = across.hypot(below);
            covers
                .layers
                .iter()
                .enumerate()
                .flat_map(move |(layer, cover)| {
                    cover
                        .pieces
                        .iter()
                        .enumerate()
                        .filter(move |(_, piece)| {
                            in_reach
                                && piece.extent.is_none_or(|extent| {
                                    from_eye <= seen.view.shown_within(extent).0 * share
                                })
                        })
                        .map(move |(piece, _)| (layer, piece, chunk))
                })
        })
    })
}

fn chunk_bounds(chunk: (u32, u32)) -> Rect<Meters> {
    let side = cover::CHUNK_CELLS as f32 / SAMPLES_PER_METER as f32;
    Rect::new(
        PaintMap::pixel_pos(chunk.0 * cover::CHUNK_CELLS, chunk.1 * cover::CHUNK_CELLS),
        Size::new(side, side),
    )
}

/// The pieces of every layer each merged region of `cut` shows: those that span a pixel at the
/// nearest its band comes to the eye.
fn shown_pieces(
    streaming: &Streaming,
    seen: Seen,
    cut: &Cut,
) -> HashMap<Merge, Vec<(usize, usize)>> {
    cut.merges
        .iter()
        .map(|&merge| {
            let nearest = streaming.regions.nearest(merge);
            let pieces = streaming
                .covers
                .layers
                .iter()
                .enumerate()
                .flat_map(|(layer, cover)| {
                    cover
                        .pieces
                        .iter()
                        .enumerate()
                        .filter(|(_, piece)| {
                            piece
                                .extent
                                .is_none_or(|extent| nearest.0 <= seen.view.shown_within(extent).0)
                        })
                        .map(move |(piece, _)| (layer, piece))
                })
                .collect();
            (merge, pieces)
        })
        .collect()
}

/// Sets the model of every prop strewn by a piece a merged region shows on its way to being ready
/// to merge: made ready off the main thread once it has loaded.
fn prepare_models(
    world: &mut World,
    streaming: &mut Streaming,
    shown: &HashMap<Merge, Vec<(usize, usize)>>,
) {
    let wanted: HashSet<(usize, usize)> = shown.values().flatten().copied().collect();
    for (layer, piece) in wanted {
        let Some(strewn) = streaming.covers.layers[layer].pieces[piece].strewn.as_mut() else {
            continue;
        };
        match &mut strewn.model {
            CopyState::Loading => {
                if let Some(pieces) = cover::model_pieces(world, &strewn.gltf) {
                    strewn.model = CopyState::Preparing(
                        AsyncComputeTaskPool::get().spawn(async move { CopyModel::of(&pieces) }),
                    );
                }
            }
            CopyState::Preparing(task) => {
                if let Some(model) = check_ready(task) {
                    strewn.model = CopyState::Ready(Arc::new(model));
                }
            }
            CopyState::Ready(_) => {}
        }
    }
}

/// Stands the merged meshes of a region's covers, once the models of the props it strews are
/// ready and its meshes are made; whether they stand.
fn stand_merge(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    seen: Seen,
    merge: Merge,
    pieces: &[(usize, usize)],
    budget: &mut Budget,
) -> bool {
    let merged = streaming
        .covers
        .merged
        .entry(merge)
        .or_insert(Merged::Waiting);
    match merged {
        Merged::Standing { rising, .. } if rising.is_empty() => return true,
        Merged::Standing { .. } => {
            let covers = &mut streaming.covers;
            let Some(Merged::Standing { stood, rising }) = covers.merged.get_mut(&merge) else {
                return false;
            };
            while rising
                .last()
                .is_some_and(|next| budget.send(mesh_bytes(&next.mesh)))
            {
                if let Some(next) = rising.pop()
                    && let Some(standing) = spawn_merged(world, &covers.layers, next, root)
                {
                    stood.push(standing);
                }
            }
            return rising.is_empty();
        }
        Merged::Merging(making) => {
            if making.made().is_none() {
                return false;
            }
            let rising = streaming
                .covers
                .merged
                .remove(&merge)
                .and_then(Merged::into_made)
                .unwrap_or_default();
            streaming.covers.merged.insert(
                merge,
                Merged::Standing {
                    stood: Vec::new(),
                    rising,
                },
            );
            return stand_merge(world, streaming, root, seen, merge, pieces, budget);
        }
        Merged::Waiting => {}
    }
    let mut far_pieces = Vec::with_capacity(pieces.len());
    for &(layer, piece) in pieces {
        let cover = &streaming.covers.layers[layer];
        let model = match cover.pieces[piece]
            .strewn
            .as_ref()
            .map(|strewn| &strewn.model)
        {
            None => None,
            Some(CopyState::Ready(model)) => Some(model.clone()),
            Some(CopyState::Loading | CopyState::Preparing(_)) => return false,
        };
        far_pieces.push(FarPiece {
            layer,
            piece,
            slot: cover.slot,
            seed: cover.seed,
            sheet_level: cover.sheet_level,
            ground_color: cover.ground_color,
            shape: cover.pieces[piece]
                .shape
                .as_ref()
                .map(|shape| shape.shape.clone()),
            spread: cover.pieces[piece].spread,
            model,
        });
    }
    if !budget.take(|budget| &mut budget.merges) {
        return false;
    }
    let region = FarRegion {
        heights: streaming.heights.clone(),
        paint: streaming.paint.clone(),
        region: merge.region,
        cells: (streaming.regions.bounds(merge.region).size.width * SAMPLES_PER_METER as f32)
            as u32,
        pixels_per_meter: seen.view.pixels_per_meter(streaming.regions.nearest(merge)),
    };
    let task = AsyncComputeTaskPool::get().spawn(async move { region.covers(&far_pieces) });
    streaming
        .covers
        .merged
        .insert(merge, Merged::Merging(Making::Task(task)));
    false
}

fn spawn_merged(
    world: &mut World,
    layers: &[CoverLayer],
    far: FarMesh,
    root: Entity,
) -> Option<Entity> {
    let piece = &layers[far.layer].pieces[far.piece];
    let material = match &piece.strewn {
        Some(strewn) => cover::merged_material(world, &strewn.material, far.material.as_ref())?,
        None => layers[far.layer].blended.clone(),
    };
    let mesh = world.resource_mut::<Assets<Mesh>>().add(far.mesh);
    Some(
        world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                far.bounds,
                NotShadowCaster,
                ChildOf(root),
            ))
            .id(),
    )
}

/// A region as its covers are merged off the main thread: the ground they grow on, which region,
/// how many cells of paint lie along its side, and how many pixels a metre takes at the nearest
/// its band comes to the eye.
struct FarRegion {
    heights: Heightfield,
    paint: PaintMap,
    region: Region,
    cells: u32,
    pixels_per_meter: f32,
}

/// A cover's piece as a region merges it.
struct FarPiece {
    layer: usize,
    piece: usize,
    slot: usize,
    seed: u64,
    sheet_level: Option<f32>,
    ground_color: LinearRgba,
    shape: Option<PieceShape>,
    spread: cover::Spread,
    model: Option<Arc<CopyModel>>,
}

impl FarRegion {
    /// The merged meshes of `pieces` over the region, together no more triangles than the region's
    /// share of the view's pixels, every piece thinned alike to keep within them.
    fn covers(&self, pieces: &[FarPiece]) -> Vec<FarMesh> {
        let (columns, rows) = self.paint.pixels();
        let first = (
            self.region.column * self.cells,
            self.region.row * self.cells,
        );
        if first.0 >= columns || first.1 >= rows {
            return Vec::new();
        }
        let shares: Vec<_> = pieces.iter().map(|far| self.painted_by(far.slot)).collect();
        let patch = |share| cover::Patch {
            heights: &self.heights,
            first,
            columns: self.cells.min(columns - first.0),
            rows: self.cells.min(rows - first.1),
            share,
        };
        let painted: Vec<f32> = shares
            .iter()
            .map(|share| painted_area(&patch(share)))
            .collect();
        let wanted: Vec<f32> = pieces
            .iter()
            .zip(&painted)
            .map(|(far, &painted)| self.wanted(far, painted))
            .collect();
        let side_pixels = self.cells as f32 / SAMPLES_PER_METER as f32 * self.pixels_per_meter;
        let budget = side_pixels * side_pixels / PIXELS_PER_TRIANGLE;
        let kept = (budget / wanted.iter().sum::<f32>().max(1.0)).min(1.0);
        pieces
            .iter()
            .zip(&shares)
            .zip(painted.into_iter().zip(wanted))
            .flat_map(|((far, share), (painted, wanted))| {
                let seed = chunk_seed(
                    far.seed,
                    [
                        far.slot as u64,
                        u64::from(far.spread.piece),
                        u64::from(self.region.row) | u64::from(self.region.level + 1) << 32,
                        u64::from(self.region.column),
                    ],
                );
                self.piece(far, &patch(share), painted, seed, wanted * kept)
                    .into_iter()
                    .filter_map(|(material, mesh)| {
                        Some(FarMesh {
                            layer: far.layer,
                            piece: far.piece,
                            material,
                            bounds: mesh.get_aabb()?,
                            mesh,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// How much of each cell the terrain in `slot` is painted over.
    fn painted_by(&self, slot: usize) -> impl Fn(u32, u32) -> f32 + '_ {
        move |column, row| {
            if usize::from(self.paint.index(column, row)) == slot {
                1.0
            } else {
                0.0
            }
        }
    }

    /// How many triangles a piece takes over `painted` square metres at its full spread.
    fn wanted(&self, far: &FarPiece, painted: f32) -> f32 {
        let copies = far.spread.per_square_meter * painted;
        match (&far.shape, &far.model) {
            (None, _) => 2.0 * (self.cells * self.cells) as f32,
            (Some(PieceShape::Blade { .. }), _) => copies,
            (Some(PieceShape::Prop { .. }), Some(model)) => {
                let size = (far.spread.size.0 + far.spread.size.1) / 2.0;
                copies * model.triangles_at(size, self.pixels_per_meter) as f32
            }
            (Some(PieceShape::Prop { .. }), None) => 0.0,
        }
    }

    /// A piece's merged meshes over `patch`, painted over `painted` square metres, spending no
    /// more than `budget` triangles.
    fn piece(
        &self,
        far: &FarPiece,
        patch: &cover::Patch,
        painted: f32,
        seed: u64,
        budget: f32,
    ) -> Vec<(Option<Handle<GltfMaterial>>, Mesh)> {
        match (&far.shape, &far.model) {
            (None, _) => {
                let quads = budget / 2.0;
                let mut stride = 1;
                while (self.cells / stride).pow(2) as f32 > quads && stride < self.cells {
                    stride *= 2;
                }
                cover::sheet(patch, far.sheet_level, far.ground_color, stride)
                    .into_iter()
                    .map(|mesh| (None, mesh))
                    .collect()
            }
            (Some(PieceShape::Blade { width, height }), _) => {
                let blades = far.spread.per_square_meter * painted;
                let spread = far.spread.thinned((budget / blades.max(1.0)).min(1.0));
                let shape = cover::BladeShape {
                    width: width.0,
                    height: height.0,
                };
                cover::blades(patch, &shape, &spread, seed)
                    .into_iter()
                    .map(|mesh| (None, mesh))
                    .collect()
            }
            (Some(PieceShape::Prop { .. }), Some(model)) => {
                let mut spread = far.spread;
                for _ in 0..3 {
                    let copies = spread.per_square_meter * painted;
                    let size = (spread.size.0 + spread.size.1) / 2.0;
                    let triangles = model.triangles_at(size, self.pixels_per_meter).max(1);
                    let share = budget / (copies.max(1.0) * triangles as f32);
                    if share >= 1.0 {
                        break;
                    }
                    spread = spread.thinned(share);
                }
                let places = cover::strew_places(patch, &spread, seed);
                cover::merged_copies(&places, model, &spread, self.pixels_per_meter)
            }
            (Some(PieceShape::Prop { .. }), None) => Vec::new(),
        }
    }
}

/// How many square metres of `patch` its cover is painted over, from every fourth cell each way.
fn painted_area(patch: &cover::Patch) -> f32 {
    const STEP: u32 = 4;
    let cell = 1.0 / SAMPLES_PER_METER as f32;
    let mut painted = 0.0;
    for row in (patch.first.1..patch.first.1 + patch.rows).step_by(STEP as usize) {
        for column in (patch.first.0..patch.first.0 + patch.columns).step_by(STEP as usize) {
            painted += (patch.share)(column, row).clamp(0.0, 1.0);
        }
    }
    painted * (STEP as f32 * cell).powi(2)
}

/// Each terrain with a cover, readied to lay it chunk by chunk as the view comes: the material
/// its sheets and blades are drawn with, through the terrain's cover shader with the area as its
/// site, and its pieces with how each is spread, a prop piece with its model loading.
fn cover_layers(world: &mut World, content: &Content, area: &Area) -> Vec<CoverLayer> {
    let bounds = area.size.bounds();
    let site = PlacedShape {
        at: Pos::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        ),
        facing: Radians::default(),
        shape: Shape::Rect {
            width: Meters(bounds.size.width),
            height: Meters(bounds.size.height),
        },
    };
    let models = content.built::<GltfInfos>();
    let prop_bounds = |prop: PropId| {
        let bounds = models.0.get(&prop.get(content).model.path())?.bounds;
        Some((bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z))
    };
    let extent = |shape: &PieceShape| match shape {
        PieceShape::Blade { width, height } => Some(width.0.max(height.0)),
        PieceShape::Prop { prop } => {
            let bounds = models.0.get(&prop.get(content).model.path())?.bounds;
            Some((bounds.max - bounds.min).max_element())
        }
    };
    let mut layers = Vec::new();
    for (slot, terrain) in area.terrains.iter().enumerate() {
        let ground = terrain.get(content);
        let Some(cover_def) = &ground.cover else {
            continue;
        };
        let Some(shader) = vfx_render::compiled_shader(world, cover_def.shader) else {
            continue;
        };
        let frame = SiteFrame::of_area(&site, |at| area.heights.height_at(at));
        let run = Run {
            progress: 0.0,
            time: Seconds(0.0),
        };
        let params = ParamsUniform::pack(cover_def.shader.get(content), |name| {
            match cover_def.params.get(name)? {
                ParamFill::Value(value) => Some(*value),
                ParamFill::Param { .. } => None,
            }
        });
        let material = |world: &mut World, strewn: Option<&cover::Spread>| {
            let weather = world.resource::<WeatherImage>().0.clone();
            world
                .resource_mut::<Assets<CoverMaterial>>()
                .add(cover::material(
                    frame.uniform(run, Stage::Cover),
                    params,
                    shader.clone(),
                    weather,
                    strewn,
                ))
        };
        let blended = material(world, None);
        let ground_color = ground.color.color().to_linear();
        let sheet = cover_def.sheet.map(|_| LayerPiece {
            shape: None,
            extent: None,
            spread: cover::Spread {
                piece: 0,
                per_square_meter: 0.0,
                size: (1.0, 1.0),
                color: (ground_color, ground_color),
            },
            strewn: None,
        });
        let pieces: Vec<LayerPiece> = sheet
            .into_iter()
            .chain(cover_def.pieces.iter().enumerate().map(|(index, piece)| {
                let spread = cover::Spread {
                    piece: index as u32,
                    per_square_meter: piece.per_square_meter(piece.shape.base_area(prop_bounds)),
                    size: (piece.size.min, piece.size.max),
                    color: (
                        piece.color.from.color().to_linear(),
                        piece.color.to.color().to_linear(),
                    ),
                };
                let strewn = match &piece.shape {
                    PieceShape::Prop { prop } => Some(Strewn {
                        gltf: assets::model(
                            world.resource::<AssetServer>(),
                            prop.get(content).model.path(),
                        ),
                        material: material(world, Some(&spread)),
                        model: CopyState::Loading,
                    }),
                    PieceShape::Blade { .. } => None,
                };
                LayerPiece {
                    extent: extent(&piece.shape).map(|extent| Meters(extent * piece.size.max)),
                    shape: Some(piece.clone()),
                    spread,
                    strewn,
                }
            }))
            .collect();
        layers.push(CoverLayer {
            slot,
            seed: cover_def.seed,
            sheet_level: cover_def
                .sheet
                .and_then(|sheet| sheet.level)
                .map(|level| level.0),
            ground_color,
            pieces,
            blended,
        });
    }
    layers
}

/// A chunk of a cover's piece, stood under `root`: its sheet or blades, or for a prop the copies
/// it strews once the prop has loaded; none where the chunk holds nothing of the piece.
fn spawn_cover_chunk(
    world: &mut World,
    streaming: &Streaming,
    layer: usize,
    piece: usize,
    chunk: (u32, u32),
    root: Entity,
) -> Option<Entity> {
    let layer = &streaming.covers.layers[layer];
    let (columns, rows) = streaming.paint.pixels();
    let share = |column: u32, row: u32| {
        if usize::from(streaming.paint.index(column, row)) == layer.slot {
            1.0
        } else {
            0.0
        }
    };
    let first = (chunk.0 * cover::CHUNK_CELLS, chunk.1 * cover::CHUNK_CELLS);
    let patch = cover::Patch {
        heights: &streaming.heights,
        first,
        columns: cover::CHUNK_CELLS.min(columns - first.0),
        rows: cover::CHUNK_CELLS.min(rows - first.1),
        share: &share,
    };
    let LayerPiece {
        shape,
        spread,
        strewn,
        ..
    } = &layer.pieces[piece];
    let seed = chunk_seed(
        layer.seed,
        [
            layer.slot as u64,
            u64::from(spread.piece),
            u64::from(chunk.1),
            u64::from(chunk.0),
        ],
    );
    let mesh = match shape.as_ref().map(|piece| &piece.shape) {
        None => cover::sheet(&patch, layer.sheet_level, layer.ground_color, 1),
        Some(PieceShape::Blade { width, height }) => cover::blades(
            &patch,
            &cover::BladeShape {
                width: width.0,
                height: height.0,
            },
            spread,
            seed,
        ),
        Some(PieceShape::Prop { .. }) => {
            let Strewn { gltf, material, .. } = strewn.as_ref()?;
            let places = cover::strew_places(&patch, spread, seed);
            if places.is_empty() {
                return None;
            }
            return Some(
                world
                    .spawn((
                        cover::Strewing {
                            gltf: gltf.clone(),
                            places,
                            material: material.clone(),
                        },
                        Transform::default(),
                        Visibility::default(),
                        ChildOf(root),
                    ))
                    .id(),
            );
        }
    }?;
    let bounds = mesh.get_aabb()?;
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    Some(
        world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(layer.blended.clone()),
                bounds,
                NotShadowCaster,
                ChildOf(root),
            ))
            .id(),
    )
}

/// A seed of its own for every chunk of every piece, mixed from the cover's seed and where the
/// chunk lies, so changing the cover's seed rescatters everything and nothing else repeats.
fn chunk_seed(seed: u64, parts: [u64; 4]) -> u64 {
    let mut mixed = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in parts {
        mixed ^= part.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
    }
    mixed
}
