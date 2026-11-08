//! The props of an area's scenery and the copies its covers strew, as the GPU draws them from the
//! scenery's arena. Every chunk the sight reaches holds its props there, and the copies of every
//! piece of its covers the view shows from as near as the sight's eyes may come: those it tells
//! apart as they are strewn, short of those the GPU thins away at that distance however near the
//! eye comes, and of a crowd it doesn't tell apart as few as the grain leaves, each grown to cover
//! as much. A chunk is worked out off the frame from the area's content and its ground once the
//! ground under it has loaded, nearest the sight first, and sent to the GPU within the frame's
//! upload budget, standing in for what stood there until it does. A chunk the sight leaves past
//! what it keeps is let go.

use std::sync::Arc;

use bevy::gltf::{Gltf, GltfMaterial};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::covers::CopyPiece;
use super::expand::{self, ChunkSpan, PaintChunk, SceneryChunk, SourcesByChunk};
use super::plan::{AHEAD, Detail, KEEP_SHARE, OFF_MIDDLE, Plan, Seen, Sight};
use super::props;
use super::{Shortfall, Shortfalls, Streaming, distance_to, ground_loaded};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::load::FileHashes;
use crate::core::ground::{Heightfield, PaintMap};
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::os::Profile;
use crate::core::render::camera::Lens;
use crate::core::render::world::cover::{self, Place};
use crate::core::render::world::gpu::arena::{
    ChunkPlace, DressedProp, InstanceArena, InstanceChange, InstanceChanges, InstanceRecord,
    ModelSize, PropModels,
};
use crate::core::render::world::gpu::atlas::{
    AtlasLayout, CANOPY_BANDS, CANOPY_EXTENT_MAX, CanopyMap, TerrainAtlas,
};
use crate::core::render::world::gpu::impostor::{
    self, AtlasKey, BakeMaterialKey, ImpostorStore, LookBakeKey, LookFromAbove,
};
use crate::core::render::world::gpu::upload;
use crate::core::render::world::lod::GRAIN_PIXELS;
use crate::core::render::world::material::{self, MaterialLook, WorldMaterial};
use crate::core::render::world_point;
use crate::core::units::{GROUND_ORIGIN, MeterSize, Meters};
use crate::core::vfx::shader::CompiledVfxShaders;
use crate::systems::area::prop::{PropDef, PropId, Scatter};
use crate::systems::area::{Area, AreaModule, PropPlacement};

/// What a scenery holds in its arena: the lots standing there, by their keys, those being worked
/// out and those worked out waiting for room in a frame's budget, the models of every prop they
/// draw, held so they stay loaded while any of them stands, what they draw dressed in its
/// materials, and the lots standing for ground since sculpted or painted anew.
pub(super) struct Instances {
    arena: InstanceArena,
    copies: Arc<[CopyPiece]>,
    /// Every prop the area stands, placed or scattered: the far field keeps those of them whose
    /// models are wide enough to show, and the canopy shows the rest from above, so every one's
    /// model is wanted until its size and its look from above are known.
    area_props: Vec<PropId>,
    /// Which of the area's prop sources reach each chunk, for the lots worked out off the frame.
    filed: Arc<SourcesByChunk>,
    /// How big each area prop's model is, as far as the models have been prepared.
    sizes: HashMap<PropId, ModelSize>,
    /// What each area prop looks like from above, as far as the impostors have been baked or read.
    looks: HashMap<PropId, LookFromAbove>,
    /// The atlas each area prop is drawn from afar by, as far as their dress is known, wanted for
    /// its look from above.
    impostor_keys: HashMap<PropId, AtlasKey>,
    canopy: Canopy,
    /// How many props the content holds: the arena draws the copies of a cover's pieces after
    /// them.
    props: u32,
    expanding: HashMap<u32, (Lot, Task<Expanded>)>,
    expanded: Vec<(u32, Lot, Expanded)>,
    models: HashMap<PropId, Handle<Gltf>>,
    dressed: HashSet<Drawn>,
    standing: HashMap<u32, Standing>,
    stale: HashSet<u32>,
    /// Counts each time the props standing change.
    props_revision: u64,
}

impl Instances {
    /// What the arena of an area holding the content's props and strewing `copies` holds.
    pub(super) fn new(content: &Content, area: &Area, copies: Arc<[CopyPiece]>) -> Instances {
        Instances {
            arena: InstanceArena::default(),
            copies,
            area_props: area_props_of(area),
            filed: Arc::new(SourcesByChunk::of(&area.prop_sources, area.size.bounds())),
            sizes: HashMap::default(),
            looks: HashMap::default(),
            impostor_keys: HashMap::default(),
            canopy: Canopy::default(),
            props: content.table::<PropDef>().len() as u32,
            expanding: HashMap::default(),
            expanded: Vec::new(),
            models: HashMap::default(),
            dressed: HashSet::default(),
            standing: HashMap::default(),
            stale: HashSet::default(),
            props_revision: 0,
        }
    }

    /// How many props stand in the arena.
    pub(super) fn props(&self) -> u32 {
        self.counted(|lot| matches!(lot, Lot::Props(_)))
    }

    /// How many copies of its covers' pieces stand in the arena.
    pub(super) fn copies(&self) -> u32 {
        self.counted(|lot| matches!(lot, Lot::Copies(..)))
    }

    /// The place among the area's prop sources of the module standing the instance at
    /// `instance` among the arena's, none for a cover's copy.
    pub(super) fn source_of(&self, instance: u32) -> Option<u32> {
        let (key, local) = self.arena.chunk_of(instance)?;
        self.standing
            .get(&key)?
            .sources
            .get(local as usize)
            .copied()
            .flatten()
    }

    /// Every instance among the arena's that a module among the area's prop sources `stood`
    /// takes stands near, where the module stands whole; the far field's are left out.
    pub(super) fn standing_for(&self, stood: impl Fn(u32) -> bool) -> Vec<u32> {
        self.standing
            .iter()
            .filter(|(_, standing)| !matches!(standing.lot, Lot::Far(..)))
            .filter_map(|(key, standing)| Some((self.arena.first_of(*key)?, standing)))
            .flat_map(|(first, standing)| {
                standing
                    .sources
                    .iter()
                    .enumerate()
                    .filter(|(_, source)| source.is_some_and(&stood))
                    .map(move |(at, _)| first + at as u32)
            })
            .collect()
    }

    /// Takes in the area's prop sources changed, the props they stood standing until what
    /// replaces them does: the lots of props on the stretches of `ground` in `touched` stand
    /// anew, and every other lot keeps standing, each of its props standing for the source
    /// `moved` tells, by its place before.
    pub(super) fn sources_changed(
        &mut self,
        area: &Area,
        ground: Rect<Meters>,
        touched: &[Rect<Meters>],
        moved: impl Fn(u32) -> Option<u32>,
    ) {
        self.area_props = area_props_of(area);
        self.filed = Arc::new(SourcesByChunk::of(&area.prop_sources, ground));
        let props = |lot: &Lot| matches!(lot, Lot::Props(_) | Lot::Far(..));
        self.expanding.retain(|_, (lot, _)| !props(lot));
        self.expanded.retain(|(_, lot, _)| !props(lot));
        for (key, standing) in &mut self.standing {
            if !props(&standing.lot) {
                continue;
            }
            for source in &mut standing.sources {
                *source = source.and_then(&moved);
            }
            let bounds = standing.lot.bounds(ground);
            if touched.iter().any(|stretch| stretch.intersects(&bounds)) {
                self.stale.insert(*key);
            }
        }
        self.props_revision += 1;
        self.canopy.sources_revision += 1;
    }

    /// A number that changes each time the props standing in the arena do.
    pub(super) fn props_revision(&self) -> u64 {
        self.props_revision
    }

    fn counted(&self, of: impl Fn(Lot) -> bool) -> u32 {
        self.standing
            .values()
            .filter(|standing| of(standing.lot))
            .map(|standing| standing.instances)
            .sum()
    }

    /// Works out anew what stands on the stretches of `ground` in `changed`, sculpted or painted
    /// anew, what stands there standing until what replaces it does.
    pub(super) fn ground_changed(&mut self, ground: Rect<Meters>, changed: &[Rect<Meters>]) {
        let touched = |lot: &Lot| {
            let bounds = lot.bounds(ground);
            changed.iter().any(|stretch| stretch.intersects(&bounds))
        };
        self.expanding.retain(|_, (lot, _)| !touched(lot));
        self.expanded.retain(|(_, lot, _)| !touched(lot));
        self.stale.extend(
            self.standing
                .iter()
                .filter(|(_, standing)| touched(&standing.lot))
                .map(|(key, _)| *key),
        );
    }
}

/// Every prop `area` stands, placed or scattered, each once.
fn area_props_of(area: &Area) -> Vec<PropId> {
    let mut props: Vec<PropId> = area
        .prop_sources
        .iter()
        .flat_map(|module| match module {
            AreaModule::Prop(placed) => vec![placed.prop],
            AreaModule::Scatter(scatter) => scatter.props.clone(),
            _ => Vec::new(),
        })
        .collect();
    props.sort_unstable();
    props.dedup();
    props
}

/// What a chunk of the arena holds: the props standing in a chunk of the ground, the copies its
/// covers strew there as a view shows them from as near as a detail, or the far field's props
/// over a tile of chunks, those wide enough to show from as far as a detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Lot {
    Props(SceneryChunk),
    Copies(SceneryChunk, Detail),
    Far(FarTile, Detail),
}

impl Lot {
    /// The ground the lot stands over.
    fn bounds(self, ground: Rect<Meters>) -> Rect<Meters> {
        match self {
            Lot::Props(chunk) | Lot::Copies(chunk, _) => chunk.bounds(ground),
            Lot::Far(tile, _) => tile.bounds(ground),
        }
    }

    /// Whether `other` stands over the same ground as this lot, at whatever detail.
    fn same_place(self, other: Lot) -> bool {
        match (self, other) {
            (Lot::Props(a), Lot::Props(b)) | (Lot::Copies(a, _), Lot::Copies(b, _)) => a == b,
            (Lot::Far(a, _), Lot::Far(b, _)) => a == b,
            _ => false,
        }
    }

    /// The two keys the lot may stand at in the arena: one for what stands, the other for what
    /// replaces it.
    fn keys(self, ground: Rect<Meters>) -> [u32; 2] {
        let (kind, index) = match self {
            Lot::Props(chunk) => (0, chunk_index(ground, chunk)),
            Lot::Copies(chunk, _) => (COPIES_KEY, chunk_index(ground, chunk)),
            Lot::Far(tile, _) => (FAR_KEY, tile.row * FarTile::across(ground) + tile.column),
        };
        [kind | index, kind | REPLACING_KEY | index]
    }
}

/// A tile of the far field: `FAR_CHUNKS` chunks of the ground a side, by its place among the
/// tiles across and down from the ground's corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FarTile {
    column: u32,
    row: u32,
}

impl FarTile {
    const SIDE: Meters = Meters(SceneryChunk::SIDE.0 * FAR_CHUNKS as f32);

    /// Every tile of `ground`, row by row.
    fn all(ground: Rect<Meters>) -> impl Iterator<Item = FarTile> {
        let across = FarTile::across(ground);
        let down = (ground.size.height / FarTile::SIDE.0).ceil() as u32;
        (0..down).flat_map(move |row| (0..across).map(move |column| FarTile { column, row }))
    }

    fn across(ground: Rect<Meters>) -> u32 {
        (ground.size.width / FarTile::SIDE.0).ceil() as u32
    }

    fn bounds(self, ground: Rect<Meters>) -> Rect<Meters> {
        Rect::new(
            Pos::new(
                ground.origin.x + self.column as f32 * FarTile::SIDE.0,
                ground.origin.y + self.row as f32 * FarTile::SIDE.0,
            ),
            Size::new(FarTile::SIDE.0, FarTile::SIDE.0),
        )
    }

    /// The chunks of `ground` the tile covers.
    fn chunks(self, ground: Rect<Meters>) -> ChunkSpan {
        let first = SceneryChunk {
            column: self.column * FAR_CHUNKS,
            row: self.row * FAR_CHUNKS,
        };
        let across = across(ground);
        let down = (ground.size.height / SceneryChunk::SIDE.0).ceil() as u32;
        ChunkSpan {
            columns: first.column..(first.column + FAR_CHUNKS).min(across),
            rows: first.row..(first.row + FAR_CHUNKS).min(down),
        }
    }
}

/// The index of `chunk` among the chunks of `ground`, row by row, as the arena's keys and the
/// far field's records name it.
fn chunk_index(ground: Rect<Meters>, chunk: SceneryChunk) -> u32 {
    chunk.row * across(ground) + chunk.column
}

const COPIES_KEY: u32 = 1 << 31;
const REPLACING_KEY: u32 = 1 << 30;
const FAR_KEY: u32 = 1 << 29;
/// The chunks a far tile spans each way.
const FAR_CHUNKS: u32 = 8;

/// A lot standing in the arena, what it draws, how many instances it holds, and for a lot of
/// props the place among the area's prop sources of the module standing each, none for one since
/// changed while the lot stands until what replaces it does.
struct Standing {
    lot: Lot,
    drawn: HashSet<Drawn>,
    instances: u32,
    sources: Vec<Option<u32>>,
    /// The key of what stood at the lot's place before, drawn on until this lot is dressed, so
    /// the place is never bare while the models it needs load.
    replacing: Option<u32>,
}

/// What an instance in the arena is drawn as: a prop, or a copy strewn by a piece of the covers,
/// by its place among the scenery's copy pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Drawn {
    Prop(PropId),
    Copy(usize),
}

impl Drawn {
    /// The prop the arena draws it as: a prop by its place among the content's, a piece's copies
    /// after all of them.
    fn id(self, props: u32) -> u32 {
        match self {
            Drawn::Prop(prop) => prop.index() as u32,
            Drawn::Copy(piece) => props + piece as u32,
        }
    }
}

/// A lot worked out: its instances, what they are drawn as, and for a lot of props the place
/// among the area's prop sources of the module standing each.
struct Expanded {
    records: Vec<InstanceRecord>,
    drawn: HashSet<Drawn>,
    sources: Vec<u32>,
}

/// How big each prop's model is, for the far field to keep the props wide enough to show: none
/// until every model of the area's props is prepared.
type Extents = Arc<HashMap<PropId, ModelSize>>;

/// The canopy over the area's ground, as far as it has come: the map being worked out for a
/// layout of the atlas and a revision of the prop sources, and the one laid last.
#[derive(Default)]
struct Canopy {
    making: Option<((AtlasLayout, u64), Task<CanopyMap>)>,
    made_for: Option<(AtlasLayout, u64)>,
    /// Counts each time the area's prop sources change.
    sources_revision: u64,
}

/// Stands in the arena every lot the plan reaches, as far as the frame's budget goes: how many of
/// them don't stand yet, by the kind of lot.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    plan: &Plan,
    root: Entity,
) -> Shortfalls {
    let ground = streaming.area.size.bounds();
    let wanted = wanted_lots(streaming, &plan.seen, ground);
    let wanted_at: HashMap<[u32; 2], Lot> =
        wanted.iter().map(|&lot| (lot.keys(ground), lot)).collect();
    let taken_in = streaming
        .sight
        .map(|sight| Reaches::of(&streaming.instances.copies, sight, sight.reach.0));
    let extents = sizes(world, streaming);
    let instances = &mut streaming.instances;
    instances
        .expanding
        .retain(|key, (lot, _)| wanted_at.get(&sibling_keys(*key)) == Some(lot));
    instances
        .expanded
        .retain(|(key, lot, _)| wanted_at.get(&sibling_keys(*key)) == Some(lot));
    let mut finished = Vec::new();
    instances
        .expanding
        .retain(|key, (lot, task)| match check_ready(task) {
            Some(expanded) => {
                finished.push((*key, *lot, expanded));
                false
            }
            None => true,
        });
    instances.expanded.extend(finished);
    send(world, streaming, root);
    dress(world, streaming, root);
    retire(world, streaming, root);
    let mut shortfalls = Shortfalls::default();
    for lot in wanted {
        let keys = lot.keys(ground);
        let instances = &streaming.instances;
        let standing_as = |key: &u32| {
            instances
                .standing
                .get(key)
                .filter(|standing| standing.lot == lot)
        };
        let dressed = |standing: &Standing| {
            standing
                .drawn
                .iter()
                .all(|drawn| instances.dressed.contains(drawn))
        };
        let fresh = keys
            .iter()
            .filter(|key| !instances.stale.contains(*key))
            .find_map(standing_as);
        let shortfall = match lot {
            Lot::Props(_) => &mut shortfalls.props,
            Lot::Copies(..) => &mut shortfalls.copies,
            Lot::Far(..) => &mut shortfalls.far,
        };
        let standing_there = |key: &u32| {
            instances
                .standing
                .get(key)
                .filter(|standing| standing.lot.same_place(lot))
        };
        let (short, missing) = if !taken_in.is_none_or(|reaches| reaches.take(lot, ground)) {
            (&mut shortfall.ahead, false)
        } else if keys.iter().filter_map(standing_as).any(dressed) {
            (&mut shortfall.stale, false)
        } else if keys.iter().filter_map(standing_there).any(dressed) {
            (&mut shortfall.ahead, false)
        } else {
            (&mut shortfall.missing, true)
        };
        if let Some(standing) = fresh {
            if !dressed(standing) {
                *short += 1;
                if missing {
                    debug!("{lot:?} missing: standing, not yet dressed");
                }
            }
            continue;
        }
        *short += 1;
        if missing {
            let state = if keys.iter().any(|key| instances.expanding.contains_key(key)) {
                "expanding"
            } else if matches!(lot, Lot::Far(..)) && extents.is_none() {
                "waiting for every model's size"
            } else {
                "not yet expanding"
            };
            debug!(
                "{lot:?} missing: {state}, {} expanding",
                instances.expanding.len()
            );
        }
        let waiting = keys.iter().any(|key| {
            instances.expanding.contains_key(key)
                || instances.expanded.iter().any(|(held, ..)| held == key)
        });
        if waiting || instances.expanding.len() >= EXPANDING_AT_ONCE {
            continue;
        }
        let extents = match lot {
            Lot::Far(..) => match &extents {
                Some(extents) => Some(extents.clone()),
                None => continue,
            },
            _ => {
                if !ground_loaded(streaming, lot.bounds(ground)) {
                    continue;
                }
                None
            }
        };
        let instances = &mut streaming.instances;
        let key = match keys
            .iter()
            .find(|key| !instances.standing.contains_key(*key))
        {
            Some(&free) => free,
            None => {
                let [first, replacing] = keys;
                let still_dressing = |key: &u32| {
                    instances
                        .standing
                        .get(key)
                        .is_some_and(|standing| standing.replacing.is_some())
                };
                let newer = if still_dressing(&first) {
                    first
                } else {
                    replacing
                };
                take_away(world, instances, root, newer);
                newer
            }
        };
        let task = expansion(streaming, &plan.seen, ground, lot, extents, key);
        streaming
            .instances
            .expanding
            .insert(key, (lot, AsyncComputeTaskPool::get().spawn(task)));
    }
    shortfalls
}

/// Lets go of every lot standing past what the plan keeps, and of any being worked out there, and
/// of the dress of everything no lot left draws, and the model too unless it is one of the area's
/// own props: those are loaded and their impostors baked as the area is readied, and are kept
/// while it stands, so a walk back to one finds its model and its atlas rather than loading and
/// baking them again.
pub(super) fn clear(world: &mut World, streaming: &mut Streaming, plan: &Plan, root: Entity) {
    let ground = streaming.area.size.bounds();
    let keep = plan.seen.reach.0 * KEEP_SHARE;
    let kept = |lot: Lot| {
        matches!(lot, Lot::Far(..)) || distance_to(lot.bounds(ground), plan.seen.at()) <= keep
    };
    let instances = &mut streaming.instances;
    instances.expanding.retain(|_, (lot, _)| kept(*lot));
    instances.expanded.retain(|(_, lot, _)| kept(*lot));
    let gone: Vec<u32> = instances
        .standing
        .iter()
        .filter(|(_, standing)| !kept(standing.lot))
        .map(|(key, _)| *key)
        .collect();
    for key in gone {
        take_away(world, instances, root, key);
    }
    let drawn: HashSet<Drawn> = instances
        .standing
        .values()
        .map(|standing| &standing.drawn)
        .chain(
            instances
                .expanded
                .iter()
                .map(|(.., expanded)| &expanded.drawn),
        )
        .flatten()
        .copied()
        .collect();
    let props: HashSet<PropId> = drawn
        .iter()
        .map(|drawn| prop_of(&instances.copies, *drawn))
        .collect();
    let Instances {
        models, area_props, ..
    } = &mut *instances;
    models.retain(|prop, _| props.contains(prop) || area_props.binary_search(prop).is_ok());
    let undressed: Vec<Drawn> = instances
        .dressed
        .extract_if(|dressed| !drawn.contains(dressed))
        .collect();
    let changes: Vec<InstanceChange> = undressed
        .into_iter()
        .map(|drawn| InstanceChange::Undressed {
            prop: drawn.id(instances.props),
        })
        .collect();
    changed(world, root, changes);
}

/// Lets the GPU go of everything the scenery under `root` held in its arena.
pub(super) fn drop_arena(world: &mut World, root: Entity) {
    changed(world, root, [InstanceChange::Dropped]);
}

/// The most lots being worked out at once.
const EXPANDING_AT_ONCE: usize = 64;

/// Every lot the sight takes in, nearest first: the props of every chunk within its reach, the
/// copies of every chunk within as far as the view shows any piece's, worked out for the detail
/// the nearest eye shows the chunk at, or the finest that thins nothing nearer than that, and
/// after them the far field: every tile of the ground not wholly within the reach and not wholly
/// past the horizon, worked out for the detail the nearest eye shows it at.
fn wanted_lots(streaming: &Streaming, seen: &Seen, ground: Rect<Meters>) -> Vec<Lot> {
    let at = seen.at();
    let reach = seen.reach.0;
    let lens = seen.lens();
    let copies = &streaming.instances.copies;
    let reaches = Reaches::of(
        copies,
        Sight {
            reach: seen.reach,
            horizon: seen.horizon,
            ..seen.sight
        },
        seen.sight.reach.0 - (OFF_MIDDLE + 2.0 * AHEAD),
    );
    let whole = seen.detail(whole_within(copies, lens).0);
    let around = Rect::new(
        at - Offset::new(reach, reach),
        Size::new(2.0 * reach, 2.0 * reach),
    );
    let mut wanted: Vec<(SceneryChunk, Lot)> = Vec::new();
    for chunk in SceneryChunk::within(ground, around) {
        let bounds = chunk.bounds(ground);
        if !reaches.props_over(bounds) {
            continue;
        }
        wanted.push((chunk, Lot::Props(chunk)));
        if !copies.is_empty() && reaches.copies_over(bounds) {
            let middle = streaming.heights.height_at(bounds.center());
            let detail = seen.detail(seen.over(bounds, middle, middle)).max(whole);
            wanted.push((chunk, Lot::Copies(chunk, detail)));
        }
    }
    wanted.sort_by(|(a, _), (b, _)| {
        let (a, b) = (
            seen.order(a.bounds(ground).center()),
            seen.order(b.bounds(ground).center()),
        );
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    let mut far: Vec<(f32, Lot)> = FarTile::all(ground)
        .filter(|tile| reaches.far_over(tile.bounds(ground)))
        .map(|tile| {
            let bounds = tile.bounds(ground);
            let middle = streaming.heights.height_at(bounds.center());
            let detail = seen.detail(seen.over(bounds, middle, middle).max(reach));
            (distance_to(bounds, at), Lot::Far(tile, detail))
        })
        .collect();
    far.sort_by(|a, b| a.0.total_cmp(&b.0));
    wanted
        .into_iter()
        .map(|(_, lot)| lot)
        .chain(far.into_iter().map(|(_, lot)| lot))
        .collect()
}

/// How far around an eye at `at` the lots of each kind are taken in: props out to the reach,
/// copies as far as the view shows their blades, and far tiles out to the horizon where they
/// reach past `far_from`: the props' reach for the eye itself, and short of it by what the plan
/// looks ahead for a plan, so a tile's far field stands before its props lots leave the reach.
#[derive(Clone, Copy)]
struct Reaches {
    at: Pos<Meters>,
    props: f32,
    copies: f32,
    far: f32,
    far_from: f32,
}

impl Reaches {
    fn of(copies: &[CopyPiece], sight: Sight, far_from: f32) -> Reaches {
        let reach = sight.reach.0;
        Reaches {
            at: sight.at,
            props: reach,
            copies: copies
                .iter()
                .map(|piece| sight.lens.shown_within(piece.largest()).0)
                .fold(0.0, f32::max)
                .min(reach),
            far: sight.horizon.map_or(f32::INFINITY, |horizon| horizon.0),
            far_from,
        }
    }

    fn take(self, lot: Lot, ground: Rect<Meters>) -> bool {
        let bounds = lot.bounds(ground);
        match lot {
            Lot::Props(_) => self.props_over(bounds),
            Lot::Copies(..) => self.copies_over(bounds),
            Lot::Far(..) => self.far_over(bounds),
        }
    }

    fn props_over(self, bounds: Rect<Meters>) -> bool {
        distance_to(bounds, self.at) <= self.props
    }

    fn copies_over(self, bounds: Rect<Meters>) -> bool {
        distance_to(bounds, self.at) <= self.copies
    }

    fn far_over(self, bounds: Rect<Meters>) -> bool {
        distance_to(bounds, self.at) <= self.far && farthest_from(bounds, self.at) > self.far_from
    }
}

/// How far the farthest point of `bounds` lies from `at`.
fn farthest_from(bounds: Rect<Meters>, at: Pos<Meters>) -> f32 {
    [
        bounds.min(),
        bounds.max(),
        Pos::new(bounds.min_x(), bounds.max_y()),
        Pos::new(bounds.max_x(), bounds.min_y()),
    ]
    .into_iter()
    .map(|corner| corner.distance_to(at))
    .fold(0.0, f32::max)
}

/// How big each model of the area's props is, once every one has been prepared; until then the
/// ones whose size isn't known yet are wanted, and kept, so each is prepared once. Each model is
/// kept as well until its look from above is known, its impostor wanted for it once the model is
/// on the GPU and dressed.
fn sizes(world: &mut World, streaming: &mut Streaming) -> Option<Extents> {
    let looks_wanted = world.contains_resource::<ImpostorStore>();
    let area_props = streaming.instances.area_props.clone();
    for prop in area_props {
        let instances = &streaming.instances;
        let known = instances.sizes.contains_key(&prop)
            && (!looks_wanted || instances.looks.contains_key(&prop));
        if known {
            continue;
        }
        let model = streaming
            .instances
            .models
            .entry(prop)
            .or_insert_with(|| {
                let path = prop.get(&streaming.content).model.path();
                let model = assets::model(world.resource::<AssetServer>(), path);
                if let Some(mut models) = world.get_resource_mut::<PropModels>() {
                    models.want(&model);
                }
                model
            })
            .clone();
        if let Some(size) = world
            .get_resource::<PropModels>()
            .and_then(|models| models.size(model.id()))
        {
            let def = prop.get(&streaming.content);
            let size = size.grown(props::overhang(&streaming.content, def));
            streaming.instances.sizes.insert(prop, size);
        }
        if !looks_wanted {
            continue;
        }
        let key = match streaming.instances.impostor_keys.get(&prop) {
            Some(key) => Some(*key),
            None => {
                let key = world
                    .get_resource::<PropModels>()
                    .and_then(|models| models.parts(model.id()))
                    .map(<[_]>::to_vec)
                    .and_then(|parts| prop_dress(world, &streaming.content, prop, &model, &parts))
                    .and_then(|dressed| dressed.impostor);
                if let Some(key) = key {
                    streaming.instances.impostor_keys.insert(prop, key);
                }
                key
            }
        };
        if let Some(key) = key
            && let Some(look) = world.resource::<ImpostorStore>().look_from_above(key)
        {
            streaming.instances.looks.insert(prop, look);
        }
    }
    let instances = &streaming.instances;
    (instances.sizes.len() == instances.area_props.len()).then(|| Arc::new(instances.sizes.clone()))
}

/// Lays the canopy over the terrain's atlas as the area's props and the atlas's layout stand now,
/// worked out off the frame once every prop's size and look from above are known: how much of it
/// has yet to be laid.
pub(super) fn lay_canopy(world: &mut World, streaming: &mut Streaming) -> Shortfall {
    if !world.contains_resource::<ImpostorStore>() {
        return Shortfall::default();
    }
    let Some(layout) = world
        .get::<TerrainAtlas>(streaming.terrain)
        .and_then(TerrainAtlas::layout)
    else {
        return Shortfall::default();
    };
    let instances = &mut streaming.instances;
    let wanted = (layout, instances.canopy.sources_revision);
    let finished = match &mut instances.canopy.making {
        Some((made_for, _)) if *made_for != wanted => Some(None),
        Some((_, task)) => check_ready(task).map(Some),
        None => None,
    };
    if let Some(made) = finished {
        instances.canopy.making = None;
        if let Some(map) = made {
            if let Some(mut atlas) = world.get_mut::<TerrainAtlas>(streaming.terrain) {
                atlas.lay_canopy(&map);
            }
            instances.canopy.made_for = Some(wanted);
        }
    }
    if instances.canopy.made_for == Some(wanted) {
        return Shortfall::default();
    }
    let known = instances.sizes.len() == instances.area_props.len()
        && instances.looks.len() == instances.area_props.len();
    if instances.canopy.making.is_none() && known {
        let area = streaming.area.clone();
        let sizes = instances.sizes.clone();
        let looks = instances.looks.clone();
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { canopy_map(&area, layout, &sizes, &looks) });
        instances.canopy.making = Some((wanted, task));
    }
    let mut shortfall = Shortfall::default();
    if instances.canopy.made_for.is_some() {
        shortfall.stale = 1;
    } else {
        shortfall.missing = 1;
    }
    shortfall
}

/// What the props of `area` look like from above over each texel of the canopy of `layout`: the
/// ground a prop hides is its look's cover of the square framing its model's sphere, at its
/// scale, laid where it stands; a scatter's over its region, as many a square metre as it strews.
/// Where more than one covers a texel their colours and extents are weighed by how much each
/// covers, and the cover tops out at the whole texel.
fn canopy_map(
    area: &Area,
    layout: AtlasLayout,
    sizes: &HashMap<PropId, ModelSize>,
    looks: &HashMap<PropId, LookFromAbove>,
) -> CanopyMap {
    let (columns, rows) = layout.canopy;
    let texel = layout.canopy_texel().0;
    let bounds = layout.canopy_bounds();
    let band_texels = CanopyMap::texels(&layout);
    let texels = band_texels * CANOPY_BANDS.len();
    let mut covers = vec![0.0f32; texels];
    let mut colors = vec![Vec3::ZERO; texels];
    let mut extents = vec![0.0f32; texels];
    let hides = |prop: PropId| -> Option<Hiding> {
        let (size, look) = (sizes.get(&prop)?, looks.get(&prop)?);
        let framed = 2.0 * size.radius.0;
        Some(Hiding {
            ground: look.cover * framed * framed,
            color: Vec3::new(look.color.red, look.color.green, look.color.blue),
            extent: size.extent.0,
        })
    };
    let mut spread = |region: Rect<Meters>, density: f32, color: Vec3, extent: f32| {
        let Some(within) = region.intersection(&bounds) else {
            return;
        };
        let band = CanopyMap::band(Meters(extent)) * band_texels;
        let first = |at: f32, origin: f32| ((at - origin) / texel).floor().max(0.0) as u32;
        let past = |at: f32, origin: f32, count: u32| {
            (((at - origin) / texel).ceil().max(0.0) as u32).min(count)
        };
        for row in first(within.min_y(), bounds.min_y())..past(within.max_y(), bounds.min_y(), rows)
        {
            for column in
                first(within.min_x(), bounds.min_x())..past(within.max_x(), bounds.min_x(), columns)
            {
                let cell = Rect::new(
                    Pos::new(
                        bounds.min_x() + column as f32 * texel,
                        bounds.min_y() + row as f32 * texel,
                    ),
                    Size::new(texel, texel),
                );
                let Some(overlap) = cell.intersection(&within) else {
                    continue;
                };
                let share = density * overlap.area() / (texel * texel);
                let at = band + (row * columns + column) as usize;
                covers[at] += share;
                colors[at] += color * share;
                extents[at] += extent * share;
            }
        }
    };
    for module in &area.prop_sources {
        match module {
            AreaModule::Prop(placed) => {
                let Some(hiding) = hides(placed.prop) else {
                    continue;
                };
                let Some(size) = sizes.get(&placed.prop) else {
                    continue;
                };
                let framed = 2.0 * size.radius.0 * placed.scale;
                let region = Rect::new(
                    Pos::new(placed.at.x - framed / 2.0, placed.at.y - framed / 2.0),
                    Size::new(framed, framed),
                );
                let cover = hiding.ground / (2.0 * size.radius.0).powi(2).max(f32::EPSILON);
                spread(region, cover, hiding.color, hiding.extent * placed.scale);
            }
            AreaModule::Scatter(scatter) => {
                let Some(hiding) = scattered_hiding(scatter, &hides) else {
                    continue;
                };
                let (min, max) = (scatter.scale.min, scatter.scale.max);
                let mean_square = (min * min + min * max + max * max) / 3.0;
                let mean = (min + max) / 2.0;
                spread(
                    scatter.region,
                    scatter.per_square_meter * hiding.ground * mean_square,
                    hiding.color,
                    hiding.extent * mean,
                );
            }
            _ => {}
        }
    }
    let mut map = CanopyMap {
        layout,
        colors: Vec::with_capacity(texels),
        extents: Vec::with_capacity(texels),
    };
    for at in 0..texels {
        let covered = covers[at];
        let color = if covered > 0.0 {
            colors[at] / covered
        } else {
            Vec3::ZERO
        };
        let srgb = Srgba::from(LinearRgba::new(color.x, color.y, color.z, 1.0));
        let byte = |share: f32| (share.clamp(0.0, 1.0) * 255.0).round() as u8;
        map.colors.push([
            byte(srgb.red),
            byte(srgb.green),
            byte(srgb.blue),
            byte(covered),
        ]);
        let extent = if covered > 0.0 {
            extents[at] / covered
        } else {
            0.0
        };
        map.extents
            .push(((extent / CANOPY_EXTENT_MAX.0).clamp(0.0, 1.0) * 65535.0).round() as u16);
    }
    map
}

/// The ground a prop hides from above at its model's size, its colour there, and how wide it is.
#[derive(Clone, Copy)]
struct Hiding {
    ground: f32,
    color: Vec3,
    extent: f32,
}

/// What `scatter`'s props hide from above, one of them on average: each weighs by the ground it
/// hides, as the scatter strews them alike.
fn scattered_hiding(
    scatter: &Scatter,
    hides: &impl Fn(PropId) -> Option<Hiding>,
) -> Option<Hiding> {
    let hidings: Vec<Hiding> = scatter
        .props
        .iter()
        .filter_map(|&prop| hides(prop))
        .collect();
    if hidings.is_empty() {
        return None;
    }
    let ground: f32 = hidings.iter().map(|hiding| hiding.ground).sum();
    let weight = ground.max(f32::EPSILON);
    Some(Hiding {
        ground: ground / hidings.len() as f32,
        color: hidings
            .iter()
            .map(|hiding| hiding.color * hiding.ground)
            .sum::<Vec3>()
            / weight,
        extent: hidings
            .iter()
            .map(|hiding| hiding.extent * hiding.ground)
            .sum::<f32>()
            / weight,
    })
}

/// How near the eye every piece's copies stand as they are strewn: each told apart from the next,
/// and none so small the GPU thins it.
fn whole_within(copies: &[CopyPiece], lens: Lens) -> Meters {
    let showing = |pixels_per_meter: f32| lens.pixels_per_meter(Meters(1.0)) / pixels_per_meter;
    Meters(
        copies
            .iter()
            .map(|piece| {
                let smallest = piece.extent.0 * piece.spread.size.0;
                let apart = GRAIN_PIXELS * piece.spread.per_square_meter.max(f32::EPSILON).sqrt();
                showing(GRAIN_PIXELS / smallest.max(f32::EPSILON)).min(showing(apart))
            })
            .fold(f32::INFINITY, f32::min),
    )
}

/// The other key of the pair `key` belongs to, with `key` itself.
fn sibling_keys(key: u32) -> [u32; 2] {
    let first = key & !REPLACING_KEY;
    [first, first | REPLACING_KEY]
}

/// The work of laying out `lot` to stand at `key`, off the frame.
fn expansion(
    streaming: &Streaming,
    seen: &Seen,
    ground: Rect<Meters>,
    lot: Lot,
    extents: Option<Extents>,
    key: u32,
) -> impl Future<Output = Expanded> + Send + 'static {
    let area = streaming.area.clone();
    let filed = streaming.instances.filed.clone();
    let heights = streaming.heights.clone();
    let paint = streaming.paint.clone();
    let copies = streaming.instances.copies.clone();
    let props = streaming.instances.props;
    let lens = seen.lens();
    async move {
        match lot {
            Lot::Props(chunk) => props_expanded(&area, &filed, &heights, ground, chunk, key),
            Lot::Far(tile, detail) => far_expanded(
                &area,
                &filed,
                &heights,
                ground,
                tile,
                &extents.unwrap_or_default(),
                GRAIN_PIXELS / lens.pixels_per_meter(detail.distance()),
            ),
            Lot::Copies(chunk, detail) => {
                let ground = Ground {
                    heights: &heights,
                    paint: &paint,
                    bounds: ground,
                };
                copies_expanded(&ground, &copies, props, chunk, (lens, detail), key)
            }
        }
    }
}

/// The props standing in `chunk` of `area`'s `ground` as the arena holds them, each on the ground
/// of `heights` under it.
fn props_expanded(
    area: &Area,
    filed: &SourcesByChunk,
    heights: &Heightfield,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
    key: u32,
) -> Expanded {
    let placed = filed.sourced_props_in(&area.prop_sources, ground, chunk);
    Expanded {
        drawn: placed
            .iter()
            .map(|(_, placed)| Drawn::Prop(placed.prop))
            .collect(),
        records: placed
            .iter()
            .map(|(_, placed)| prop_record(heights, placed, key))
            .collect(),
        sources: placed.iter().map(|(source, _)| *source).collect(),
    }
}

/// The props of the far field standing in `tile` of `area`'s `ground` as the arena holds them:
/// those whose model, at the scale placed, spans at least `least` metres, each on the ground of
/// `heights` under it and carrying the index of the chunk it lies in.
fn far_expanded(
    area: &Area,
    filed: &SourcesByChunk,
    heights: &Heightfield,
    ground: Rect<Meters>,
    tile: FarTile,
    extents: &HashMap<PropId, ModelSize>,
    least: f32,
) -> Expanded {
    let mut expanded = Expanded {
        records: Vec::new(),
        drawn: HashSet::default(),
        sources: Vec::new(),
    };
    let span = tile.chunks(ground);
    for (source, chunk, placed) in filed.sourced_props_within(&area.prop_sources, ground, &span) {
        let wide = extents
            .get(&placed.prop)
            .is_some_and(|size| size.extent.0 * placed.scale >= least);
        if !wide {
            continue;
        }
        expanded.drawn.insert(Drawn::Prop(placed.prop));
        expanded
            .records
            .push(prop_record(heights, &placed, chunk_index(ground, chunk)));
        expanded.sources.push(source);
    }
    expanded
}

/// The ground copies are strewn over: its heights, its paint, and the bounds of all of it.
struct Ground<'a> {
    heights: &'a Heightfield,
    paint: &'a PaintMap,
    bounds: Rect<Meters>,
}

/// The copies the pieces of `copies` strew in `chunk` of `ground` as a view through `lens` shows
/// them at `detail`, as the arena holds them after the content's `props`: of a piece the view
/// tells apart, every copy it strews short of those the GPU thins away there; of one it doesn't,
/// as few as the grain leaves, each grown to cover as much; and nothing of a piece too small to
/// show.
fn copies_expanded(
    ground: &Ground,
    copies: &[CopyPiece],
    props: u32,
    chunk: SceneryChunk,
    (lens, detail): (Lens, Detail),
    key: u32,
) -> Expanded {
    let bounds = chunk.bounds(ground.bounds);
    let pixels_per_meter = lens.pixels_per_meter(detail.distance());
    let mut records = Vec::new();
    let mut drawn = HashSet::default();
    for (index, piece) in copies.iter().enumerate() {
        let extent = piece.extent.0;
        if detail.distance().0 > lens.shown_within(piece.largest()).0 {
            continue;
        }
        let told_apart = piece.spread.told_apart(extent, pixels_per_meter);
        let spread = if told_apart {
            piece.spread
        } else {
            piece.spread.grained(extent, pixels_per_meter)
        };
        let before = records.len();
        for paint in paint_chunks(bounds) {
            let places =
                expand::strewn_in(ground.heights, ground.paint, piece.cover, &spread, paint);
            records.extend(
                places
                    .iter()
                    .filter(|place| !told_apart || kept(place, extent, pixels_per_meter))
                    .map(|place| copy_record(place, Drawn::Copy(index).id(props), key)),
            );
        }
        if records.len() > before {
            drawn.insert(Drawn::Copy(index));
        }
    }
    Expanded {
        records,
        drawn,
        sources: Vec::new(),
    }
}

/// Whether the GPU draws the copy at `place`, `extent` across at its size, anywhere as far off the
/// eye as a view at `pixels_per_meter` shows it or farther: one too small for the grain is kept by
/// its seed, as one in as many as it would be grown to stand in for.
fn kept(place: &Place, extent: f32, pixels_per_meter: f32) -> bool {
    let across = extent * place.scale * pixels_per_meter;
    if across >= GRAIN_PIXELS {
        return true;
    }
    let grown = GRAIN_PIXELS / across.max(f32::EPSILON);
    place.seed < 1.0 / (grown * grown)
}

/// The chunks of paint whose first cell lies in `bounds`, so every chunk of paint is strewn by
/// one chunk of the scenery alone.
fn paint_chunks(bounds: Rect<Meters>) -> impl Iterator<Item = PaintChunk> {
    let side = PaintChunk::SIDE.0;
    let first = |at: f32, origin: f32| ((at - origin) / side).ceil().max(0.0) as u32;
    let origin = GROUND_ORIGIN;
    let columns = first(bounds.min_x(), origin.x)..first(bounds.max_x(), origin.x);
    let rows = first(bounds.min_y(), origin.y)..first(bounds.max_y(), origin.y);
    rows.flat_map(move |row| {
        columns
            .clone()
            .map(move |column| PaintChunk { column, row })
    })
}

/// Sends the lots worked out to the arena, nearest first as they were asked for, as many as the
/// frame's budget holds, wanting the models of what they draw, each to take the place of what
/// stands at its pair's other key once it is dressed.
fn send(world: &mut World, streaming: &mut Streaming, root: Entity) {
    while let Some((_, _, expanded)) = streaming.instances.expanded.first() {
        let bytes = expanded.records.len() * InstanceRecord::BYTES as usize;
        if !upload::spend(world, bytes) {
            return;
        }
        let (key, lot, expanded) = streaming.instances.expanded.remove(0);
        let mut models_of = HashMap::new();
        for &drawn in &expanded.drawn {
            let prop = prop_of(&streaming.instances.copies, drawn);
            let model = streaming
                .instances
                .models
                .entry(prop)
                .or_insert_with(|| {
                    let path = prop.get(&streaming.content).model.path();
                    let model = assets::model(world.resource::<AssetServer>(), path);
                    if let Some(mut models) = world.get_resource_mut::<PropModels>() {
                        models.want(&model);
                    }
                    model
                })
                .id();
            models_of.insert(drawn.id(streaming.instances.props), model);
        }
        let instances = &mut streaming.instances;
        let place = match lot {
            Lot::Props(chunk) | Lot::Copies(chunk, _) => ChunkPlace::Near {
                index: chunk_index(streaming.area.size.bounds(), chunk),
            },
            Lot::Far(..) => ChunkPlace::Far,
        };
        let placed = instances
            .arena
            .insert(key, &expanded.records, |prop| models_of[&prop], place);
        instances.stale.remove(&key);
        if matches!(lot, Lot::Props(_) | Lot::Far(..)) {
            instances.props_revision += 1;
        }
        let [first, replacing] = sibling_keys(key);
        let replaced = if key == first { replacing } else { first };
        instances.standing.insert(
            key,
            Standing {
                lot,
                drawn: expanded.drawn,
                instances: expanded.records.len() as u32,
                sources: expanded.sources.into_iter().map(Some).collect(),
                replacing: instances
                    .standing
                    .contains_key(&replaced)
                    .then_some(replaced),
            },
        );
        changed(world, root, placed);
    }
}

/// Lets go of what stood at a place once what replaces it there stands dressed.
fn retire(world: &mut World, streaming: &mut Streaming, root: Entity) {
    let instances = &streaming.instances;
    let retired: Vec<(u32, u32)> = instances
        .standing
        .iter()
        .filter(|(_, standing)| {
            standing
                .drawn
                .iter()
                .all(|drawn| instances.dressed.contains(drawn))
        })
        .filter_map(|(key, standing)| standing.replacing.map(|replaced| (*key, replaced)))
        .collect();
    let instances = &mut streaming.instances;
    for (key, replaced) in retired {
        take_away(world, instances, root, replaced);
        if let Some(standing) = instances.standing.get_mut(&key) {
            standing.replacing = None;
        }
    }
}

/// Lets go of what stands at `key` in the arena.
fn take_away(world: &mut World, instances: &mut Instances, root: Entity, key: u32) {
    instances.stale.remove(&key);
    if let Some(standing) = instances.standing.remove(&key) {
        if matches!(standing.lot, Lot::Props(_) | Lot::Far(..)) {
            instances.props_revision += 1;
        }
        changed(world, root, instances.arena.remove(key));
    }
}

/// Dresses everything the arena draws whose model is on the GPU, once every material it is drawn
/// in is ready: a prop each part of its model in the world material its glTF material and the
/// prop's looks give it, once every look's shader is compiled; a cover's copies each part in the
/// piece's material in the part's texture, once the texture has loaded.
fn dress(world: &mut World, streaming: &mut Streaming, root: Entity) {
    let instances = &streaming.instances;
    let undressed: Vec<Drawn> = instances
        .standing
        .values()
        .flat_map(|standing| &standing.drawn)
        .filter(|drawn| !instances.dressed.contains(*drawn))
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    for drawn in undressed {
        let instances = &streaming.instances;
        let prop = prop_of(&instances.copies, drawn);
        let Some(model) = instances.models.get(&prop).cloned() else {
            continue;
        };
        let Some(parts) = world
            .get_resource::<PropModels>()
            .and_then(|models| models.parts(model.id()))
            .map(<[_]>::to_vec)
        else {
            continue;
        };
        let dressed = match drawn {
            Drawn::Prop(prop) => prop_dress(world, &streaming.content, prop, &model, &parts),
            Drawn::Copy(piece) => copy_dress(world, &instances.copies[piece], &parts),
        };
        let Some(dressed) = dressed else {
            continue;
        };
        streaming.instances.dressed.insert(drawn);
        let prop = drawn.id(streaming.instances.props);
        changed(
            world,
            root,
            [InstanceChange::Dressed {
                prop,
                dressed: Arc::new(dressed),
            }],
        );
    }
}

/// How the arena draws `prop` of `content`, its `model` drawn in `parts`.
fn prop_dress(
    world: &mut World,
    content: &Content,
    prop: PropId,
    model: &Handle<Gltf>,
    parts: &[Option<Handle<GltfMaterial>>],
) -> Option<DressedProp> {
    let def = prop.get(content);
    let looks = props::compiled_looks(world, content, def)?;
    let names: HashMap<AssetId<GltfMaterial>, String> = world
        .resource::<Assets<Gltf>>()
        .get(model.id())
        .map(|loaded| {
            loaded
                .named_materials
                .iter()
                .map(|(name, material)| (material.id(), name.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let looks: Vec<Option<&MaterialLook>> = parts
        .iter()
        .map(|material| {
            material
                .as_ref()
                .and_then(|material| names.get(&material.id()))
                .and_then(|name| looks.iter().find(|look| look.material == *name))
        })
        .collect();
    let materials: Option<Vec<Handle<WorldMaterial>>> = parts
        .iter()
        .zip(&looks)
        .map(|(material, look)| material::part_material(world, model, material.as_ref(), *look))
        .collect();
    let materials = materials?;
    let overhang = props::overhang(content, def);
    let impostor = want_impostor(
        world, content, def, model, parts, &looks, &materials, overhang,
    );
    let translucency = looks
        .iter()
        .flatten()
        .map(|look| look.translucency)
        .fold(0.0, f32::max);
    Some(DressedProp {
        materials: materials.into_iter().map(Handle::untyped).collect(),
        shadow: def.shadow,
        fades: def.fade,
        overhang,
        translucency,
        impostor,
    })
}

/// Wants the atlas of `model` dressed by `materials`, the world materials its parts are drawn in
/// under `looks`, framed `overhang` past the model: read from the file kept for a model made of
/// the same and dressed alike, else baked. The atlas's key.
#[allow(clippy::too_many_arguments)]
fn want_impostor(
    world: &mut World,
    content: &Content,
    def: &PropDef,
    model: &Handle<Gltf>,
    parts: &[Option<Handle<GltfMaterial>>],
    looks: &[Option<&MaterialLook>],
    materials: &[Handle<WorldMaterial>],
    overhang: Meters,
) -> Option<AtlasKey> {
    let path = model.path().cloned()?;
    let keys: Vec<BakeMaterialKey> = parts
        .iter()
        .zip(looks)
        .map(|(material, look)| {
            let label = match material {
                Some(material) => material
                    .path()
                    .and_then(|path| path.label())
                    .map(|label| format!("{label}/std")),
                None => Some(format!("{}/std", GltfAssetLabel::DefaultMaterial)),
            };
            let standard = label
                .and_then(|label| {
                    world
                        .resource::<AssetServer>()
                        .get_handle::<StandardMaterial>(path.clone().with_label(label))
                })
                .and_then(|handle| world.resource::<Assets<StandardMaterial>>().get(&handle));
            let look = look.and_then(|look| {
                let shader = def.materials.get(&look.material)?.get(content).shader;
                let source = content.built::<CompiledVfxShaders>().0.get(&shader)?;
                Some(LookBakeKey {
                    shader: blake3::hash(source.as_bytes()).to_hex().to_string(),
                    params: look.params.bits(),
                    translucency: look.translucency,
                })
            });
            match standard {
                Some(standard) => BakeMaterialKey {
                    color: standard.base_color.to_linear().to_f32_array(),
                    texture: standard
                        .base_color_texture
                        .as_ref()
                        .and_then(|texture| texture.path())
                        .map(ToString::to_string),
                    cutoff: match standard.alpha_mode {
                        AlphaMode::Opaque => 0.0,
                        AlphaMode::Mask(cutoff) => cutoff,
                        _ => 0.5,
                    },
                    look,
                },
                None => BakeMaterialKey {
                    color: [1.0; 4],
                    texture: None,
                    cutoff: 0.0,
                    look,
                },
            }
        })
        .collect();
    let model_hash = content
        .built::<FileHashes>()
        .0
        .get(def.model.as_str())
        .map(|hash| hash.hash.as_str())
        .unwrap_or_default();
    let key = impostor::atlas_key(model_hash, overhang, &keys);
    if world.contains_resource::<ImpostorStore>() {
        let profile = world.resource::<Profile>().clone();
        let materials: Arc<[UntypedHandle]> =
            materials.iter().cloned().map(Handle::untyped).collect();
        world
            .resource_mut::<ImpostorStore>()
            .want(&profile, model.id(), key, materials, overhang);
    }
    Some(key)
}

/// How the arena draws the copies `piece` strews, its prop's model drawn in `parts`: casting no
/// shadow, and never clearing.
fn copy_dress(
    world: &mut World,
    piece: &CopyPiece,
    parts: &[Option<Handle<GltfMaterial>>],
) -> Option<DressedProp> {
    let loaded = parts.iter().flatten().all(|material| {
        let texture = world
            .resource::<Assets<GltfMaterial>>()
            .get(material)
            .and_then(|material| material.base_color_texture.clone());
        texture.is_none_or(|texture| {
            world
                .resource::<AssetServer>()
                .is_loaded_with_dependencies(&texture)
        })
    });
    if !loaded {
        return None;
    }
    let materials: Option<Vec<UntypedHandle>> = parts
        .iter()
        .map(|material| {
            cover::copy_material(world, &piece.material, material.as_ref()).map(Handle::untyped)
        })
        .collect();
    Some(DressedProp {
        materials: materials?,
        shadow: false,
        fades: false,
        overhang: Meters(0.0),
        translucency: 0.0,
        impostor: None,
    })
}

/// The prop `drawn` is a model of.
fn prop_of(copies: &[CopyPiece], drawn: Drawn) -> PropId {
    match drawn {
        Drawn::Prop(prop) => prop,
        Drawn::Copy(piece) => copies[piece].prop,
    }
}

/// Hands the render world `changes` to the arena of the scenery under `root`.
fn changed(world: &mut World, root: Entity, changes: impl IntoIterator<Item = InstanceChange>) {
    if let Some(mut held) = world.get_resource_mut::<InstanceChanges>() {
        held.0
            .extend(changes.into_iter().map(|change| (root, change)));
    }
}

/// The instance of `placed`, standing as [`super::prop_transform`] stands its scene on the ground
/// of `heights`, with a seed drawn from where it stands.
fn prop_record(heights: &Heightfield, placed: &PropPlacement, chunk: u32) -> InstanceRecord {
    let at = world_point(placed.at, heights.height_at(placed.at) - placed.sunk);
    InstanceRecord {
        position: at.to_array(),
        yaw: -placed.yaw.radians().0,
        scale: placed.scale,
        prop: placed.prop.index() as u32,
        seed: placed.at.x.to_bits().rotate_left(16) ^ placed.at.y.to_bits(),
        chunk,
    }
}

/// The instance of the copy at `place`, drawn as the arena's prop `prop`: its seed in the upper
/// bits of the instance's, as the GPU thins it by, and its tint in the lowest byte.
fn copy_record(place: &Place, prop: u32, chunk: u32) -> InstanceRecord {
    let seed = ((place.seed * SEED_STEPS) as u32).min(SEED_STEPS as u32 - 1);
    let tint = (place.tint.clamp(0.0, 1.0) * 255.0).round() as u32;
    InstanceRecord {
        position: place.at.to_array(),
        yaw: place.yaw.0,
        scale: place.scale,
        prop,
        seed: seed << 8 | tint,
        chunk,
    }
}

/// The steps a copy's seed takes in the upper bits of its instance's.
const SEED_STEPS: f32 = 16_777_216.0;

fn across(ground: Rect<Meters>) -> u32 {
    (ground.size.width / SceneryChunk::SIDE.0).ceil().max(1.0) as u32
}
