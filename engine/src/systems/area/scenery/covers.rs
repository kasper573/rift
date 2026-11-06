//! The covers an area's terrains grow, over the regions the scenery is cut into for the view. In
//! a leaf a cover stands the copies of the props it strews chunk by chunk, each copy an instance of
//! its prop. In every coarser region it stands them merged, a mesh for each of its pieces, of the
//! same copies where the chunks would strew them, each no finer than the view shows it: copies too
//! small to tell apart thinned and the rest grown to cover as much, so a view over a whole area
//! stands a few meshes, and what stands merged looks as what would stand chunk by chunk. A cover's
//! sheet stands with the ground, drawn over its patches wherever its terrain paints, and the GPU
//! grows its blades round the views. A prop whose copies the view shows large
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

use super::expand::{self, PaintChunk, StrewnCover, painted_by};
use super::plan::{KEEP_SHARE, Plan, Planner, Seen};
use super::regions::{Cut, Merge, Region, Unstood};
use super::{Budget, Making, Merged, PLACEMENT_CELL, Streaming, distance_to, take_away};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use crate::core::math::Pos;
use crate::core::render::camera::Lens;
use crate::core::render::world::cover::{self, CopyModel, CoverMaterial};
use crate::core::render::world::gpu::draw::GroundSheet;
use crate::core::render::world::gpu::grass::{BladeShape, GrassPiece};
use crate::core::render::world::gpu::upload;
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

/// A chunk of a cover's piece.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CoverChunk {
    piece: PieceAt,
    chunk: PaintChunk,
}

/// A cover a terrain grows, ready to stand: its slot among the area's terrains, what its sheet lies
/// on if it has one and the terrain's colour it is drawn in, its pieces, the material its sheet and
/// blades are drawn with, and the cover's seed.
struct CoverLayer {
    slot: usize,
    seed: u64,
    sheet: Option<CoverSheet>,
    ground_color: LinearRgba,
    pieces: Vec<LayerPiece>,
    grass: Vec<GrassPiece>,
    blended: Handle<CoverMaterial>,
}

/// A piece of a cover strewing a prop, as its layer stands it: its shape and how it is spread, the
/// most a copy of it measures across where that is known, and the model it strews and the
/// material its copies are drawn with.
struct LayerPiece {
    shape: CoverPiece,
    extent: Option<Meters>,
    spread: cover::Spread,
    strewn: Strewn,
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

    /// The pieces of the covers that grow blades.
    pub(super) fn grass(&self) -> Arc<[GrassPiece]> {
        self.layers
            .iter()
            .flat_map(|layer| layer.grass.iter().cloned())
            .collect()
    }

    /// The sheets the covers lay over the ground.
    pub(super) fn sheets(&self) -> Vec<GroundSheet> {
        self.layers
            .iter()
            .filter_map(|layer| {
                Some(GroundSheet {
                    terrain: u8::try_from(layer.slot).ok()?,
                    on: match layer.sheet? {
                        CoverSheet::Ground => cover::SheetOn::Ground,
                        CoverSheet::Water => cover::SheetOn::Water,
                    },
                    color: layer.ground_color,
                    material: layer.blended.clone(),
                })
            })
            .collect()
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

/// Every cover's pieces as a plan measures them, and how many pixels the paint they grow on holds
/// across and down.
pub(super) struct CoverPieces {
    layers: Vec<Vec<PieceSize>>,
    paint: (u32, u32),
}

/// How a cover's piece measures: the most a copy of it measures across where that is known, and
/// how far across its smallest copies measure.
struct PieceSize {
    extent: Option<Meters>,
    smallest: Option<Meters>,
}

impl Covers {
    /// The covers' pieces as a plan measures them, growing on `paint`.
    pub(super) fn pieces(&self, paint: &PaintMap) -> CoverPieces {
        CoverPieces {
            layers: self
                .layers
                .iter()
                .map(|layer| {
                    layer
                        .pieces
                        .iter()
                        .map(|piece| PieceSize {
                            extent: piece.extent,
                            smallest: piece.smallest(),
                        })
                        .collect()
                })
                .collect(),
            paint: paint.pixels(),
        }
    }
}

/// What a plan stands of the covers: the chunks of the pieces near the view, in the order they
/// stand, those of them that stay where they stand already, each merged region with the pieces
/// it stands merged, and every piece a merged region shows.
#[derive(Default)]
pub(super) struct CoversPlan {
    near: Vec<CoverChunk>,
    kept: HashSet<CoverChunk>,
    merging: Vec<(Merge, Vec<PieceAt>)>,
    shown: HashSet<PieceAt>,
}

/// What `cut` stands of the covers, seen from `seen`: the chunks of the leaves, and the merged
/// meshes of every coarser region.
pub(super) fn plan(planner: &Planner, heights: &Heightfield, seen: &Seen, cut: &Cut) -> CoversPlan {
    let pieces = &planner.pieces;
    if pieces.layers.is_empty() {
        return CoversPlan::default();
    }
    let lens = seen.lens();
    let shown = shown_pieces(pieces, lens, cut);
    let chunked = chunked_pieces(pieces, lens, &shown);
    let mut near: Vec<CoverChunk> =
        near_chunks(pieces, heights, seen, cut, &chunked, 1.0).collect();
    near.sort_by(|a, b| {
        let (a, b) = (
            seen.order(a.chunk.bounds().center()),
            seen.order(b.chunk.bounds().center()),
        );
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    let kept = near_chunks(pieces, heights, seen, cut, &chunked, KEEP_SHARE).collect();
    let merging = shown
        .iter()
        .map(|(merge, pieces)| {
            let merged = pieces
                .iter()
                .filter(|piece| {
                    !chunked
                        .get(merge)
                        .is_some_and(|chunked| chunked.contains(piece))
                })
                .copied()
                .collect();
            (*merge, merged)
        })
        .collect();
    CoversPlan {
        near,
        kept,
        merging,
        shown: shown.into_values().flatten().collect(),
    }
}

/// Stands what `plan` takes in of the covers: the chunks of the leaves, and the merged meshes of
/// every coarser region; where it doesn't stand yet.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    planner: &Planner,
    plan: &Plan,
    root: Entity,
    budget: &mut Budget,
) -> Unstood {
    let mut unstood = Unstood::default();
    if streaming.covers.layers.is_empty() {
        return unstood;
    }
    prepare_models(world, streaming, &plan.covers.shown);
    let mut unstood_chunks: HashSet<PaintChunk> = HashSet::default();
    let mut missing = Vec::new();
    for &key in &plan.covers.near {
        match streaming.covers.chunks.get(&key) {
            None => missing.push(key),
            Some(parts) if parts.iter().any(|&part| unshown(world, part)) => {
                unstood.add(key.chunk.bounds());
            }
            Some(_) => {}
        }
    }
    for key in missing {
        let loaded = super::ground_loaded(streaming, key.chunk.bounds());
        if !loaded
            || !unstood_chunks.is_empty()
            || !budget.stage()
            || !budget.take(|budget| &mut budget.covers)
        {
            unstood_chunks.insert(key.chunk);
            continue;
        }
        let parts = spawn_cover_chunk(world, streaming, key, root);
        if parts.iter().any(|&part| unshown(world, part)) {
            unstood.add(key.chunk.bounds());
        }
        streaming.covers.chunks.insert(key, parts);
    }
    for chunk in unstood_chunks {
        unstood.add(chunk.bounds());
    }
    let lens = plan.seen.lens();
    for (merge, merged) in &plan.covers.merging {
        if !stand_merge(
            world,
            streaming,
            planner,
            root,
            lens,
            (*merge, merged),
            budget,
        ) {
            unstood.add(planner.regions.bounds(merge.region));
        }
    }
    unstood
}

/// Takes down what stood of the covers for an earlier plan and stands for none in `plan`,
/// wherever what stands for `plan` does, and what stood for ground sculpted or painted anew once
/// all of it does. A merge for an earlier plan that shows nothing yet is given up at once.
pub(super) fn clear(
    world: &mut World,
    streaming: &mut Streaming,
    planner: &Planner,
    plan: &Plan,
    unstood: &Unstood,
) {
    let wanted: HashSet<Merge> = plan
        .covers
        .merging
        .iter()
        .map(|(merge, _)| *merge)
        .collect();
    let covers = &mut streaming.covers;
    if unstood.is_empty() {
        for stale in covers.stale.drain(..) {
            take_away(world, stale);
        }
    }
    covers.chunks.retain(|key, chunk| {
        let keep = plan.covers.kept.contains(key) || !unstood.stands_over(key.chunk.bounds());
        if !keep {
            for &part in chunk.iter() {
                take_away(world, part);
            }
        }
        keep
    });
    let regions = &planner.regions;
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

/// Whether a part of a cover's chunk is yet to show whole: its copies still being strewn, or its
/// meshes not yet on the GPU.
fn unshown(world: &World, part: Entity) -> bool {
    cover::strewing(world, part) || !upload::sent(world, part)
}

/// Every chunk of every piece in the leaves of `cut` the view takes in, or would within `share`
/// times its reach and its pixels' distance, and every chunk of the merged regions' pieces that
/// are `chunked` there.
fn near_chunks<'a>(
    pieces: &'a CoverPieces,
    heights: &'a Heightfield,
    seen: &'a Seen,
    cut: &'a Cut,
    chunked: &'a HashMap<Merge, Vec<PieceAt>>,
    share: f32,
) -> impl Iterator<Item = CoverChunk> + 'a {
    let lens = seen.lens();
    let in_reach =
        move |chunk: PaintChunk| distance_to(chunk.bounds(), seen.at()) <= seen.reach.0 * share;
    let leaves = cut.leaves.iter().flat_map(move |&leaf| {
        region_chunks(pieces.paint, leaf)
            .filter(move |&chunk| in_reach(chunk))
            .flat_map(move |chunk| {
                let bounds = chunk.bounds();
                let ground = heights.height_at(bounds.center());
                let shown_at = seen.detail(seen.over(bounds, ground, ground)).distance();
                pieces
                    .layers
                    .iter()
                    .enumerate()
                    .flat_map(move |(layer, sizes)| {
                        sizes
                            .iter()
                            .enumerate()
                            .filter(move |(_, size)| {
                                size.extent.is_none_or(|extent| {
                                    shown_at.0 <= lens.shown_within(extent).0 * share
                                })
                            })
                            .map(move |(piece, _)| CoverChunk {
                                piece: PieceAt { layer, piece },
                                chunk,
                            })
                    })
            })
    });
    let merged = chunked.iter().flat_map(move |(merge, pieces_at)| {
        region_chunks(pieces.paint, merge.region)
            .filter(move |&chunk| in_reach(chunk))
            .flat_map(move |chunk| {
                pieces_at
                    .iter()
                    .map(move |&piece| CoverChunk { piece, chunk })
            })
    });
    leaves.chain(merged)
}

/// The chunks of paint `pixels` across and down within `region`.
fn region_chunks(pixels: (u32, u32), region: Region) -> impl Iterator<Item = PaintChunk> {
    let (columns, rows) = pixels;
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

/// The pieces of every layer each merged region of `cut` shows: those that span a pixel through
/// `lens` at the detail the region shows at.
fn shown_pieces(pieces: &CoverPieces, lens: Lens, cut: &Cut) -> HashMap<Merge, Vec<PieceAt>> {
    cut.merges
        .iter()
        .map(|&merge| {
            let nearest = merge.detail.distance();
            let shown = pieces
                .layers
                .iter()
                .enumerate()
                .flat_map(|(layer, sizes)| {
                    sizes
                        .iter()
                        .enumerate()
                        .filter(|(_, size)| {
                            size.extent
                                .is_none_or(|extent| nearest.0 <= lens.shown_within(extent).0)
                        })
                        .map(move |(piece, _)| PieceAt { layer, piece })
                })
                .collect();
            (merge, shown)
        })
        .collect()
}

/// Sets the model of every prop strewn by a piece a merged region shows on its way to being ready
/// to merge: made ready off the main thread once it has loaded.
fn prepare_models(world: &mut World, streaming: &mut Streaming, shown: &HashSet<PieceAt>) {
    for &PieceAt { layer, piece } in shown {
        let strewn = &mut streaming.covers.layers[layer].pieces[piece].strewn;
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

/// The pieces of `shown` that each merged region strews chunk by chunk, as a leaf does: those
/// strewing a prop whose smallest copies span `STREWN_PIXELS` there through `lens`.
fn chunked_pieces(
    pieces: &CoverPieces,
    lens: Lens,
    shown: &HashMap<Merge, Vec<PieceAt>>,
) -> HashMap<Merge, Vec<PieceAt>> {
    shown
        .iter()
        .map(|(&merge, shown)| {
            let pixels_per_meter = lens.pixels_per_meter(merge.detail.distance());
            let chunked = shown
                .iter()
                .filter(|&&PieceAt { layer, piece }| {
                    let size = &pieces.layers[layer][piece];
                    size.smallest
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
    planner: &Planner,
    root: Entity,
    lens: Lens,
    (merge, pieces): (Merge, &[PieceAt]),
    budget: &mut Budget,
) -> bool {
    let covers = &mut streaming.covers;
    let merged = covers.merged.entry(merge).or_insert(Merged::Waiting(()));
    if let Some(stands) = merged.rise(world, |world, next| {
        spawn_merged(world, &covers.layers, next, root)
    }) {
        return stands;
    }
    let mut far_pieces = Vec::with_capacity(pieces.len());
    for &at in pieces {
        let PieceAt { layer, piece } = at;
        let cover = &streaming.covers.layers[layer];
        let model = match &cover.pieces[piece].strewn.model {
            CopyState::Ready(model) => Some(model.clone()),
            CopyState::Loading | CopyState::Preparing(_) => return false,
        };
        far_pieces.push(FarPiece {
            piece: at,
            slot: cover.slot,
            seed: cover.seed,
            shape: cover.pieces[piece].shape.shape.clone(),
            spread: cover.pieces[piece].spread,
            model,
        });
    }
    if !budget.stage() || !budget.take(|budget| &mut budget.merges) {
        return false;
    }
    let region = FarRegion {
        heights: streaming.heights.clone(),
        paint: streaming.paint.clone(),
        region: merge.region,
        cells: (planner.regions.bounds(merge.region).size.width * SAMPLES_PER_METER as f32) as u32,
        pixels_per_meter: lens.pixels_per_meter(merge.detail.distance()),
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
    let material = cover::merged_material(world, &piece.strewn.material, far.material.as_ref())?;
    let mesh = upload::stage(world, far.mesh);
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
    piece: PieceAt,
    slot: usize,
    seed: u64,
    shape: PieceShape,
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
                let seed = expand::mixed_seed(
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
            let cover = StrewnCover {
                slot: far.slot,
                seed: far.seed,
            };
            let places = chunks_within(first, end)
                .flat_map(|chunk| {
                    let places =
                        expand::strewn_in(&self.heights, &self.paint, cover, &far.spread, chunk);
                    cover::seen_at(&places, extent, self.pixels_per_meter)
                })
                .collect::<Vec<_>>();
            (places, far.spread)
        };
        let meshes: Vec<(Option<Handle<GltfMaterial>>, Mesh)> = match (&far.shape, &far.model) {
            (PieceShape::Blade { .. }, _) => Vec::new(),
            (PieceShape::Prop { .. }, Some(model)) => {
                let (places, spread) = strewn(model.extent());
                cover::merged_copies(&places, model, &spread, self.pixels_per_meter)
            }
            (PieceShape::Prop { .. }, None) => Vec::new(),
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
}

/// The chunks of paint from the cell at `first` to the one before `end`.
fn chunks_within(first: (u32, u32), end: (u32, u32)) -> impl Iterator<Item = PaintChunk> {
    let side = cover::CHUNK_CELLS;
    (first.1 / side..end.1.div_ceil(side)).flat_map(move |row| {
        (first.0 / side..end.0.div_ceil(side)).map(move |column| PaintChunk { column, row })
    })
}

/// Each terrain with a cover, readied to lay it chunk by chunk as the view comes: the material
/// its sheet and blades are drawn with, through the terrain's cover shader with the area as its
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
        let mut pieces = Vec::new();
        let mut grass = Vec::new();
        for (index, piece) in cover_def.pieces.iter().enumerate() {
            let spread = cover::Spread {
                piece: index as u32,
                per_square_meter: piece.per_square_meter(piece.shape.base_area(prop_bounds)),
                size: (piece.size.min, piece.size.max),
                color: (
                    piece.color.from.color().to_linear(),
                    piece.color.to.color().to_linear(),
                ),
            };
            match &piece.shape {
                PieceShape::Prop { prop } => pieces.push(LayerPiece {
                    extent: extent(&piece.shape).map(|extent| Meters(extent * piece.size.max)),
                    shape: piece.clone(),
                    strewn: Strewn {
                        gltf: assets::model(
                            world.resource::<AssetServer>(),
                            prop.get(content).model.path(),
                        ),
                        material: material(world, Some(&spread)),
                        model: CopyState::Loading,
                    },
                    spread,
                }),
                PieceShape::Blade { width, height } => {
                    if let Ok(terrain) = u8::try_from(slot) {
                        grass.push(GrassPiece {
                            terrain,
                            piece: spread.piece,
                            seed: cover_def.seed,
                            per_square_meter: spread.per_square_meter,
                            size: spread.size,
                            color: spread.color,
                            shape: BladeShape {
                                width: width.0,
                                height: height.0,
                            },
                            material: blended.clone(),
                        });
                    }
                }
            }
        }
        layers.push(CoverLayer {
            slot,
            seed: cover_def.seed,
            sheet: cover_def.sheet,
            ground_color,
            pieces,
            grass,
            blended,
        });
    }
    layers
}

/// A chunk of a cover's piece, stood under `root`: its blades, or for a prop the copies
/// it strews once the prop has loaded; nothing where the chunk holds nothing of the piece.
fn spawn_cover_chunk(
    world: &mut World,
    streaming: &Streaming,
    CoverChunk { piece, chunk }: CoverChunk,
    root: Entity,
) -> Vec<Entity> {
    let PieceAt { layer, piece } = piece;
    let layer = &streaming.covers.layers[layer];
    let LayerPiece { spread, strewn, .. } = &layer.pieces[piece];
    let cover = StrewnCover {
        slot: layer.slot,
        seed: layer.seed,
    };
    let places = expand::strewn_in(&streaming.heights, &streaming.paint, cover, spread, chunk);
    if places.is_empty() {
        return Vec::new();
    }
    vec![
        world
            .spawn((
                cover::Strewing {
                    gltf: strewn.gltf.clone(),
                    places,
                    material: strewn.material.clone(),
                },
                Transform::default(),
                Visibility::default(),
                ChildOf(root),
            ))
            .id(),
    ]
}
