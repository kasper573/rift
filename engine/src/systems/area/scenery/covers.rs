//! The covers an area's terrains grow, over the regions the scenery is cut into for the view. In
//! a leaf a cover stands chunk by chunk: its sheet, its blades and the copies of the props it
//! strews, each copy an instance of its prop. In every coarser region it stands merged, a mesh for
//! each of its pieces, of the same copies where the chunks would strew them, each no finer than
//! the view shows it: a sheet no finer than its pixels, and copies too small to tell apart thinned
//! and the rest grown to cover as much, so a view over a whole area stands a few meshes, and what
//! stands merged looks as what would stand chunk by chunk. A prop whose copies the view shows large
//! there stands chunk by chunk too, and a coarser region stands its covers only once the view may
//! see it, so ground out of sight costs nothing however far the scenery reaches.

use std::sync::Arc;

use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::gltf::{Gltf, GltfMaterial};
use bevy::light::NotShadowCaster;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::futures_lite::future;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::regions::{Cut, Merge, Region, Unstood};
use super::{
    Budget, KEEP_SHARE, Making, Merged, PLACEMENT_CELL, Seen, Streaming, distance_to, mesh_bytes,
    take_away,
};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER, WaterSurface};
use crate::core::math::{Pos, Rect, Size};
use crate::core::render::camera::ground_pos;
use crate::core::render::world::cover::{self, CopyModel, CoverMaterial};
use crate::core::render::world::lod::GRAIN_PIXELS;
use crate::core::render::world_point;
use crate::core::units::{MeterSize, Meters, Radians, Seconds};
use crate::core::vfx::ParamFill;
use crate::core::vfx::render::{self as vfx_render, Run, SiteFrame};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::uniform::{ParamsUniform, WeatherImage};
use crate::systems::area::Area;
use crate::systems::area::prop::PropId;
use crate::systems::area::terrain::{CoverPiece, CoverSheet, PieceShape};

/// The covers of an area's terrains as its scenery stands them: each terrain's that has one,
/// what stands of them chunk by chunk, and what stands of them merged.
pub(super) struct Covers {
    layers: Vec<CoverLayer>,
    chunks: HashMap<CoverChunk, Vec<Entity>>,
    /// What stands of each merged region, waiting first on the models of the props its covers
    /// strew.
    merged: HashMap<Merge, Merged<(), FarMesh>>,
    /// What stood for ground since sculpted or painted anew, standing until what stands for the
    /// ground as it is now does.
    stale: Vec<Entity>,
}

/// How many pixels across a prop's copies span at the least, in a region the view shows them, to
/// stand there chunk by chunk rather than merged: few enough of them fill a view to stand one by
/// one, each an instance of the prop's own mesh, where merging would make and send the GPU
/// meshes of all their triangles again.
const STREWN_PIXELS: f32 = 16.0;

/// A cover's piece: its layer among the area's covers and its place among the layer's pieces.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PieceAt {
    layer: usize,
    piece: usize,
}

/// A chunk of the paint, by its place among the chunks across and down.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PaintChunk {
    column: u32,
    row: u32,
}

impl PaintChunk {
    /// The cell of paint the chunk begins at.
    fn first_cell(self) -> (u32, u32) {
        (
            self.column * cover::CHUNK_CELLS,
            self.row * cover::CHUNK_CELLS,
        )
    }

    fn bounds(self) -> Rect<Meters> {
        let side = cover::CHUNK_CELLS as f32 / SAMPLES_PER_METER as f32;
        let (column, row) = self.first_cell();
        Rect::new(PaintMap::pixel_pos(column, row), Size::new(side, side))
    }
}

/// A chunk of a cover's piece.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CoverChunk {
    piece: PieceAt,
    chunk: PaintChunk,
}

/// A cover a terrain grows, ready to stand: its slot among the area's terrains, its pieces, the
/// material its sheets and blades are drawn with, and the cover's seed.
struct CoverLayer {
    slot: usize,
    seed: u64,
    sheet: Option<CoverSheet>,
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

impl LayerPiece {
    /// How far across the piece's smallest copies or blades measure.
    fn smallest(&self) -> Option<Meters> {
        let (smallest, largest) = self.spread.size;
        self.extent
            .map(|extent| Meters(extent.0 * smallest / largest))
    }
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

/// A merged mesh of a cover's piece over a region, and for a strewn prop the material of its
/// model it is drawn in.
struct FarMesh {
    piece: PieceAt,
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
            .extend(self.chunks.drain().flat_map(|(_, chunk)| chunk));
        for (_, merged) in self.merged.drain() {
            self.stale.extend(merged.stood());
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
    let shown = shown_pieces(streaming, seen, cut);
    prepare_models(world, streaming, &shown);
    let chunked = chunked_pieces(streaming, seen, &shown);
    let mut unstood_chunks: HashSet<PaintChunk> = HashSet::default();
    let mut missing = Vec::new();
    for key in near_chunks(streaming, seen, cut, &chunked, 1.0) {
        match streaming.covers.chunks.get(&key) {
            None => missing.push(key),
            Some(parts) if parts.iter().any(|&part| cover::strewing(world, part)) => {
                unstood.add(key.chunk.bounds());
            }
            Some(_) => {}
        }
    }
    for key in missing {
        if !unstood_chunks.is_empty() || !budget.take(|budget| &mut budget.covers) {
            unstood_chunks.insert(key.chunk);
            continue;
        }
        let parts = spawn_cover_chunk(world, streaming, key, root);
        if parts.iter().any(|&part| cover::strewing(world, part)) {
            unstood.add(key.chunk.bounds());
        }
        streaming.covers.chunks.insert(key, parts);
    }
    for chunk in unstood_chunks {
        unstood.add(chunk.bounds());
    }
    let merging: Vec<(Merge, Vec<PieceAt>)> = shown
        .into_iter()
        .filter(|(merge, _)| in_view(streaming, seen, merge.region))
        .map(|(merge, pieces)| {
            let merged = pieces
                .into_iter()
                .filter(|piece| {
                    !chunked
                        .get(&merge)
                        .is_some_and(|chunked| chunked.contains(piece))
                })
                .collect();
            (merge, merged)
        })
        .collect();
    for (merge, merged) in merging {
        if !stand_merge(world, streaming, root, seen, merge, &merged, budget) {
            unstood.add(streaming.regions.bounds(merge.region));
        }
    }
    unstood
}

/// Takes down what stood of the covers for an earlier view and stands for none in `cut`, wherever
/// what stands for `cut` does, and what stood for ground sculpted or painted anew once all of it
/// does. A merge for an earlier view that shows nothing yet is given up at once.
pub(super) fn clear(
    world: &mut World,
    streaming: &mut Streaming,
    seen: Seen,
    cut: &Cut,
    unstood: &Unstood,
) {
    let chunked = chunked_pieces(streaming, seen, &shown_pieces(streaming, seen, cut));
    let kept: HashSet<CoverChunk> =
        near_chunks(streaming, seen, cut, &chunked, KEEP_SHARE).collect();
    let wanted: HashSet<Merge> = cut
        .merges
        .iter()
        .filter(|merge| in_view(streaming, seen, merge.region))
        .copied()
        .collect();
    let covers = &mut streaming.covers;
    if unstood.is_empty() {
        for stale in covers.stale.drain(..) {
            take_away(world, stale);
        }
    }
    covers.chunks.retain(|key, chunk| {
        let keep = kept.contains(key) || !unstood.stands_over(key.chunk.bounds());
        if !keep {
            for &part in chunk.iter() {
                take_away(world, part);
            }
        }
        keep
    });
    let regions = &streaming.regions;
    covers.merged.retain(|merge, merged| {
        let keep = wanted.contains(merge)
            || !merged.stood().is_empty() && !unstood.stands_over(regions.bounds(merge.region));
        if !keep {
            for &mesh in merged.stood() {
                take_away(world, mesh);
            }
        }
        keep
    });
}

/// Every chunk of every piece in the leaves of `cut` the view takes in, or would within `share`
/// times its reach and its pixels' distance, and every chunk of the merged regions' pieces that
/// are `chunked` there.
fn near_chunks<'a>(
    streaming: &'a Streaming,
    seen: Seen,
    cut: &'a Cut,
    chunked: &'a HashMap<Merge, Vec<PieceAt>>,
    share: f32,
) -> impl Iterator<Item = CoverChunk> + 'a {
    let eye = seen.view.eye();
    let covers = &streaming.covers;
    let in_reach =
        move |chunk: PaintChunk| distance_to(chunk.bounds(), seen.at) <= seen.reach.0 * share;
    let leaves = cut.leaves.iter().flat_map(move |&leaf| {
        region_chunks(streaming, leaf)
            .filter(move |&chunk| in_reach(chunk))
            .flat_map(move |chunk| {
                let bounds = chunk.bounds();
                let across = distance_to(bounds, ground_pos(eye));
                let below = eye.y - streaming.heights.height_at(bounds.center()).0;
                let shown_at = seen.detail(across.hypot(below)).distance();
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
                                piece.extent.is_none_or(|extent| {
                                    shown_at.0 <= seen.view.shown_within(extent).0 * share
                                })
                            })
                            .map(move |(piece, _)| CoverChunk {
                                piece: PieceAt { layer, piece },
                                chunk,
                            })
                    })
            })
    });
    let merged = chunked.iter().flat_map(move |(merge, pieces)| {
        region_chunks(streaming, merge.region)
            .filter(move |&chunk| in_reach(chunk))
            .flat_map(move |chunk| pieces.iter().map(move |&piece| CoverChunk { piece, chunk }))
    });
    leaves.chain(merged)
}

/// The chunks of paint within `region`.
fn region_chunks(streaming: &Streaming, region: Region) -> impl Iterator<Item = PaintChunk> {
    let (columns, rows) = streaming.paint.pixels();
    let side = (((PLACEMENT_CELL.0 * SAMPLES_PER_METER as f32) as u32) << region.level)
        / cover::CHUNK_CELLS;
    let (chunk_columns, chunk_rows) = (
        columns.div_ceil(cover::CHUNK_CELLS),
        rows.div_ceil(cover::CHUNK_CELLS),
    );
    (region.row * side..((region.row + 1) * side).min(chunk_rows)).flat_map(move |row| {
        (region.column * side..((region.column + 1) * side).min(chunk_columns))
            .map(move |column| PaintChunk { column, row })
    })
}

/// Whether the view may see any of the covers over `region`.
fn in_view(streaming: &Streaming, seen: Seen, region: Region) -> bool {
    let bounds = streaming.regions.bounds(region);
    let heights = streaming.heights.range();
    let tallest = streaming
        .covers
        .layers
        .iter()
        .flat_map(|layer| &layer.pieces)
        .filter_map(|piece| piece.extent)
        .fold(0.0, |tallest: f32, extent| tallest.max(extent.0));
    seen.view.may_see(
        world_point(bounds.min(), heights.lowest),
        world_point(bounds.max(), Meters(heights.highest.0 + tallest)),
    )
}

/// The pieces of every layer each merged region of `cut` shows: those that span a pixel at the
/// detail the view shows it at.
fn shown_pieces(streaming: &Streaming, seen: Seen, cut: &Cut) -> HashMap<Merge, Vec<PieceAt>> {
    cut.merges
        .iter()
        .map(|&merge| {
            let nearest = merge.detail.distance();
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
                        .map(move |(piece, _)| PieceAt { layer, piece })
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
    shown: &HashMap<Merge, Vec<PieceAt>>,
) {
    let wanted: HashSet<PieceAt> = shown.values().flatten().copied().collect();
    for PieceAt { layer, piece } in wanted {
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

/// The pieces of `shown` that each merged region the view may see strews chunk by chunk, as a leaf
/// does: those strewing a prop whose smallest copies span `STREWN_PIXELS` there.
fn chunked_pieces(
    streaming: &Streaming,
    seen: Seen,
    shown: &HashMap<Merge, Vec<PieceAt>>,
) -> HashMap<Merge, Vec<PieceAt>> {
    shown
        .iter()
        .filter(|(merge, _)| in_view(streaming, seen, merge.region))
        .map(|(&merge, pieces)| {
            let pixels_per_meter = seen.view.pixels_per_meter(merge.detail.distance());
            let chunked = pieces
                .iter()
                .filter(|&&PieceAt { layer, piece }| {
                    let piece = &streaming.covers.layers[layer].pieces[piece];
                    piece.strewn.is_some()
                        && piece
                            .smallest()
                            .is_some_and(|smallest| smallest.0 * pixels_per_meter >= STREWN_PIXELS)
                })
                .copied()
                .collect();
            (merge, chunked)
        })
        .collect()
}

/// Stands the merged meshes of a region's covers, once the models of the props it strews are
/// ready and its meshes are made; whether they stand.
fn stand_merge(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    seen: Seen,
    merge: Merge,
    pieces: &[PieceAt],
    budget: &mut Budget,
) -> bool {
    let covers = &mut streaming.covers;
    let merged = covers.merged.entry(merge).or_insert(Merged::Waiting(()));
    if let Some(stands) = merged.rise(
        budget,
        |next| mesh_bytes(&next.mesh),
        |next| spawn_merged(world, &covers.layers, next, root),
    ) {
        return stands;
    }
    let mut far_pieces = Vec::with_capacity(pieces.len());
    for &at in pieces {
        let PieceAt { layer, piece } = at;
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
            piece: at,
            slot: cover.slot,
            seed: cover.seed,
            sheet: cover.sheet,
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
        water: streaming.water.clone(),
        region: merge.region,
        cells: (streaming.regions.bounds(merge.region).size.width * SAMPLES_PER_METER as f32)
            as u32,
        pixels_per_meter: seen.view.pixels_per_meter(merge.detail.distance()),
    };
    let task = AsyncComputeTaskPool::get().spawn(async move { region.covers(&far_pieces).await });
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
    let PieceAt { layer, piece } = far.piece;
    let piece = &layers[layer].pieces[piece];
    let material = match &piece.strewn {
        Some(strewn) => cover::merged_material(world, &strewn.material, far.material.as_ref())?,
        None => layers[layer].blended.clone(),
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
    water: WaterSurface,
    region: Region,
    cells: u32,
    pixels_per_meter: f32,
}

/// A cover's piece as a region merges it.
struct FarPiece {
    piece: PieceAt,
    slot: usize,
    seed: u64,
    sheet: Option<CoverSheet>,
    ground_color: LinearRgba,
    shape: Option<PieceShape>,
    spread: cover::Spread,
    model: Option<Arc<CopyModel>>,
}

impl FarRegion {
    /// The merged meshes of `pieces` over the region as the view shows them there: every copy
    /// where a chunk near the eye strews it, at its own size, and only those too small for the
    /// view to tell apart thinned; a piece whose copies the view doesn't tell apart, too small or
    /// too close together, strewn thinned from the start, as where such copies stand doesn't show.
    /// Gives way to the tasks waiting behind it after every piece, as a region takes long to
    /// merge, so a view's other merges aren't held up behind it, and one no view wants any more
    /// stops there.
    async fn covers(&self, pieces: &[FarPiece]) -> Vec<FarMesh> {
        let (columns, rows) = self.paint.pixels();
        let first = (
            self.region.column * self.cells,
            self.region.row * self.cells,
        );
        if first.0 >= columns || first.1 >= rows {
            return Vec::new();
        }
        let end = (
            (first.0 + self.cells).min(columns),
            (first.1 + self.cells).min(rows),
        );
        let mut made = Vec::new();
        for far in pieces {
            made.extend(self.piece(far, first, end));
            future::yield_now().await;
        }
        made
    }

    /// The merged meshes of a piece over the region's cells from `first` to the one before `end`.
    fn piece(&self, far: &FarPiece, first: (u32, u32), end: (u32, u32)) -> Vec<FarMesh> {
        let share = painted_by(&self.paint, far.slot);
        let patch = |first: (u32, u32), side: u32| cover::Patch {
            heights: &self.heights,
            first,
            columns: side.min(end.0 - first.0),
            rows: side.min(end.1 - first.1),
            share: &share,
        };
        let strewn = |extent: f32| {
            if !far.spread.told_apart(extent, self.pixels_per_meter) {
                let seed = chunk_seed(
                    far.seed,
                    [
                        far.slot as u64,
                        u64::from(far.spread.piece),
                        u64::from(self.region.row) | u64::from(self.region.level + 1) << 32,
                        u64::from(self.region.column),
                    ],
                );
                let spread = far.spread.grained(extent, self.pixels_per_meter);
                return (
                    cover::strew_places(&patch(first, self.cells), &spread, seed),
                    spread,
                );
            }
            let places = chunks_within(first, end)
                .flat_map(|chunk| {
                    let seed = chunk_seed(
                        far.seed,
                        [
                            far.slot as u64,
                            u64::from(far.spread.piece),
                            u64::from(chunk.row),
                            u64::from(chunk.column),
                        ],
                    );
                    let first = chunk.first_cell();
                    let places =
                        cover::strew_places(&patch(first, cover::CHUNK_CELLS), &far.spread, seed);
                    cover::seen_at(&places, extent, self.pixels_per_meter)
                })
                .collect::<Vec<_>>();
            (places, far.spread)
        };
        let meshes: Vec<(Option<Handle<GltfMaterial>>, Mesh)> = match (&far.shape, &far.model) {
            (None, _) => cover::sheet(
                &patch(first, self.cells),
                sheet_on(far.sheet, &self.water),
                far.ground_color,
                self.sheet_stride(),
            )
            .into_iter()
            .map(|mesh| (None, mesh))
            .collect(),
            (Some(PieceShape::Blade { width, height }), _) => {
                let (places, spread) = strewn(width.0.max(height.0));
                let shape = cover::BladeShape {
                    width: width.0,
                    height: height.0,
                };
                cover::blades(&self.heights, &places, &shape, &spread)
                    .into_iter()
                    .map(|mesh| (None, mesh))
                    .collect()
            }
            (Some(PieceShape::Prop { .. }), Some(model)) => {
                let (places, spread) = strewn(model.extent());
                cover::merged_copies(&places, model, &spread, self.pixels_per_meter)
            }
            (Some(PieceShape::Prop { .. }), None) => Vec::new(),
        };
        meshes
            .into_iter()
            .filter_map(|(material, mesh)| {
                Some(FarMesh {
                    piece: far.piece,
                    material,
                    bounds: mesh.get_aabb()?,
                    mesh,
                })
            })
            .collect()
    }

    /// How many cells of paint a quad of a sheet spans: as many as the view tells apart no better
    /// than one.
    fn sheet_stride(&self) -> u32 {
        let cell = self.pixels_per_meter / SAMPLES_PER_METER as f32;
        let mut stride = 1;
        while (stride * 2) as f32 * cell <= GRAIN_PIXELS && stride * 2 <= self.cells {
            stride *= 2;
        }
        stride
    }
}

/// The chunks of paint from the cell at `first` to the one before `end`.
fn chunks_within(first: (u32, u32), end: (u32, u32)) -> impl Iterator<Item = PaintChunk> {
    let side = cover::CHUNK_CELLS;
    (first.1 / side..end.1.div_ceil(side)).flat_map(move |row| {
        (first.0 / side..end.0.div_ceil(side)).map(move |column| PaintChunk { column, row })
    })
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
            sheet: cover_def.sheet,
            ground_color,
            pieces,
            blended,
        });
    }
    layers
}

/// A chunk of a cover's piece, stood under `root`: its sheet or blades, or for a prop the copies
/// it strews once the prop has loaded; nothing where the chunk holds nothing of the piece.
fn spawn_cover_chunk(
    world: &mut World,
    streaming: &Streaming,
    CoverChunk { piece, chunk }: CoverChunk,
    root: Entity,
) -> Vec<Entity> {
    let PieceAt { layer, piece } = piece;
    let layer = &streaming.covers.layers[layer];
    let (columns, rows) = streaming.paint.pixels();
    let share = painted_by(&streaming.paint, layer.slot);
    let first = chunk.first_cell();
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
            u64::from(chunk.row),
            u64::from(chunk.column),
        ],
    );
    let meshes = match shape.as_ref().map(|piece| &piece.shape) {
        None => cover::sheet(
            &patch,
            sheet_on(layer.sheet, &streaming.water),
            layer.ground_color,
            1,
        ),
        Some(PieceShape::Blade { width, height }) => cover::blades(
            &streaming.heights,
            &cover::strew_places(&patch, spread, seed),
            &cover::BladeShape {
                width: width.0,
                height: height.0,
            },
            spread,
        ),
        Some(PieceShape::Prop { .. }) => {
            let Some(Strewn { gltf, material, .. }) = strewn.as_ref() else {
                return Vec::new();
            };
            let places = cover::strew_places(&patch, spread, seed);
            if places.is_empty() {
                return Vec::new();
            }
            return vec![
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
            ];
        }
    };
    meshes
        .into_iter()
        .filter_map(|mesh| {
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
        })
        .collect()
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

/// The share of each cell of paint that `slot` paints: all of it or none.
fn painted_by(paint: &PaintMap, slot: usize) -> impl Fn(u32, u32) -> f32 + '_ {
    move |column, row| {
        if usize::from(paint.index(column, row)) == slot {
            1.0
        } else {
            0.0
        }
    }
}

/// What the sheet of a cover lying `sheet` lies on, over the area's `water`.
fn sheet_on(sheet: Option<CoverSheet>, water: &WaterSurface) -> cover::SheetOn<'_> {
    match sheet {
        Some(CoverSheet::Water) => cover::SheetOn::Water(water),
        Some(CoverSheet::Ground) | None => cover::SheetOn::Ground,
    }
}
