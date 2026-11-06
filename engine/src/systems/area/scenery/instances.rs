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
use super::expand::{self, PaintChunk, SceneryChunk};
use super::plan::{Detail, KEEP_SHARE, Plan, Seen};
use super::props;
use super::{Budget, Streaming, distance_to, ground_loaded};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::ground::{Heightfield, PaintMap};
use crate::core::math::{Offset, Rect, Size};
use crate::core::render::camera::Lens;
use crate::core::render::world::cover::{self, Place};
use crate::core::render::world::gpu::arena::{
    DressedProp, InstanceArena, InstanceChange, InstanceChanges, InstanceRecord, PropModels,
};
use crate::core::render::world::gpu::upload;
use crate::core::render::world::lod::GRAIN_PIXELS;
use crate::core::render::world::material::{self, WorldMaterial};
use crate::core::render::world_point;
use crate::core::units::{GROUND_ORIGIN, MeterSize, Meters};
use crate::systems::area::prop::{PropDef, PropId};
use crate::systems::area::{Area, PropPlacement};

/// What a scenery holds in its arena: the lots standing there, by their keys, those being worked
/// out and those worked out waiting for room in a frame's budget, the models of every prop they
/// draw, held so they stay loaded while any of them stands, what they draw dressed in its
/// materials, and the lots standing for ground since sculpted or painted anew.
pub(super) struct Instances {
    arena: InstanceArena,
    copies: Arc<[CopyPiece]>,
    /// How many props the content holds: the arena draws the copies of a cover's pieces after
    /// them.
    props: u32,
    expanding: HashMap<u32, (Lot, Task<Expanded>)>,
    expanded: Vec<(u32, Lot, Expanded)>,
    models: HashMap<PropId, Handle<Gltf>>,
    dressed: HashSet<Drawn>,
    standing: HashMap<u32, Standing>,
    stale: HashSet<u32>,
}

impl Instances {
    /// What the arena of an area holding the content's props and strewing `copies` holds.
    pub(super) fn new(content: &Content, copies: Arc<[CopyPiece]>) -> Instances {
        Instances {
            arena: InstanceArena::default(),
            copies,
            props: content.table::<PropDef>().len() as u32,
            expanding: HashMap::default(),
            expanded: Vec::new(),
            models: HashMap::default(),
            dressed: HashSet::default(),
            standing: HashMap::default(),
            stale: HashSet::default(),
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

    fn counted(&self, of: impl Fn(Lot) -> bool) -> u32 {
        self.standing
            .values()
            .filter(|standing| of(standing.lot))
            .map(|standing| standing.instances)
            .sum()
    }

    /// Works everything out again over ground sculpted or painted anew, what stands standing
    /// until what replaces it does.
    pub(super) fn ground_changed(&mut self) {
        self.expanding.clear();
        self.expanded.clear();
        self.stale.extend(self.standing.keys().copied());
    }
}

/// What a chunk of the arena holds: the props standing in a chunk of the ground, or the copies its
/// covers strew there as a view shows them from as near as a detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Lot {
    Props(SceneryChunk),
    Copies(SceneryChunk, Detail),
}

impl Lot {
    fn chunk(self) -> SceneryChunk {
        match self {
            Lot::Props(chunk) | Lot::Copies(chunk, _) => chunk,
        }
    }

    /// The two keys the lot may stand at in the arena: one for what stands, the other for what
    /// replaces it.
    fn keys(self, ground: Rect<Meters>) -> [u32; 2] {
        let chunk = self.chunk();
        let index = chunk.row * across(ground) + chunk.column;
        let kind = match self {
            Lot::Props(_) => 0,
            Lot::Copies(..) => COPIES_KEY,
        };
        [kind | index, kind | REPLACING_KEY | index]
    }
}

const COPIES_KEY: u32 = 1 << 31;
const REPLACING_KEY: u32 = 1 << 30;

/// A lot standing in the arena, what it draws, and how many instances it holds.
struct Standing {
    lot: Lot,
    drawn: HashSet<Drawn>,
    instances: u32,
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

/// A lot worked out: its instances, and what they are drawn as.
struct Expanded {
    records: Vec<InstanceRecord>,
    drawn: HashSet<Drawn>,
}

/// Stands in the arena every lot the plan reaches, as far as the frame's budget goes: how many of
/// them don't stand yet.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    plan: &Plan,
    root: Entity,
    budget: &mut Budget,
) -> u32 {
    let ground = streaming.area.size.bounds();
    let wanted = wanted_lots(streaming, &plan.seen, ground);
    let wanted_at: HashMap<[u32; 2], Lot> =
        wanted.iter().map(|&lot| (lot.keys(ground), lot)).collect();
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
    let mut missing = 0;
    for lot in wanted {
        let keys = lot.keys(ground);
        let instances = &streaming.instances;
        let standing = keys.iter().find(|key| {
            !instances.stale.contains(*key)
                && instances
                    .standing
                    .get(*key)
                    .is_some_and(|standing| standing.lot == lot)
        });
        if let Some(key) = standing {
            let drawn = &instances.standing[key].drawn;
            if !drawn.iter().all(|drawn| instances.dressed.contains(drawn)) {
                missing += 1;
            }
            continue;
        }
        missing += 1;
        let waiting = keys.iter().any(|key| {
            instances.expanding.contains_key(key)
                || instances.expanded.iter().any(|(held, ..)| held == key)
        });
        if waiting || instances.expanding.len() >= EXPANDING_AT_ONCE {
            continue;
        }
        if !ground_loaded(streaming, lot.chunk().bounds(ground)) {
            continue;
        }
        let instances = &mut streaming.instances;
        let key = match keys
            .iter()
            .find(|key| !instances.standing.contains_key(*key))
        {
            Some(&free) => free,
            None => {
                let [_, replacing] = keys;
                take_away(world, instances, root, replacing);
                replacing
            }
        };
        let task = expansion(streaming, &plan.seen, ground, lot, key);
        streaming
            .instances
            .expanding
            .insert(key, (lot, AsyncComputeTaskPool::get().spawn(task)));
    }
    if missing > 0 {
        budget.defer();
    }
    missing
}

/// Lets go of every lot standing past what the plan keeps, and of any being worked out there, and
/// of the model and the dress of everything no lot left draws.
pub(super) fn clear(world: &mut World, streaming: &mut Streaming, plan: &Plan, root: Entity) {
    let ground = streaming.area.size.bounds();
    let keep = plan.seen.reach.0 * KEEP_SHARE;
    let kept = |chunk: SceneryChunk| distance_to(chunk.bounds(ground), plan.seen.at()) <= keep;
    let instances = &mut streaming.instances;
    instances.expanding.retain(|_, (lot, _)| kept(lot.chunk()));
    instances.expanded.retain(|(_, lot, _)| kept(lot.chunk()));
    let gone: Vec<u32> = instances
        .standing
        .iter()
        .filter(|(_, standing)| !kept(standing.lot.chunk()))
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
    instances.models.retain(|prop, _| props.contains(prop));
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
const EXPANDING_AT_ONCE: usize = 8;

/// Every lot the sight takes in, nearest first: the props of every chunk within its reach, and
/// the copies of every chunk within as far as the view shows any piece's, worked out for the
/// detail the nearest eye shows the chunk at, or the finest that thins nothing nearer than that.
fn wanted_lots(streaming: &Streaming, seen: &Seen, ground: Rect<Meters>) -> Vec<Lot> {
    let at = seen.at();
    let reach = seen.reach.0;
    let lens = seen.lens();
    let copies = &streaming.instances.copies;
    let shown = copies
        .iter()
        .map(|piece| lens.shown_within(piece.largest()).0)
        .fold(0.0, f32::max)
        .min(reach);
    let whole = seen.detail(whole_within(copies, lens).0);
    let around = Rect::new(
        at - Offset::new(reach, reach),
        Size::new(2.0 * reach, 2.0 * reach),
    );
    let mut wanted: Vec<(SceneryChunk, Lot)> = Vec::new();
    for chunk in SceneryChunk::within(ground, around) {
        let bounds = chunk.bounds(ground);
        let apart = distance_to(bounds, at);
        if apart > reach {
            continue;
        }
        wanted.push((chunk, Lot::Props(chunk)));
        if !copies.is_empty() && apart <= shown {
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
    wanted.into_iter().map(|(_, lot)| lot).collect()
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
    key: u32,
) -> impl Future<Output = Expanded> + Send + 'static {
    let area = streaming.area.clone();
    let heights = streaming.heights.clone();
    let paint = streaming.paint.clone();
    let copies = streaming.instances.copies.clone();
    let props = streaming.instances.props;
    let lens = seen.lens();
    async move {
        match lot {
            Lot::Props(chunk) => props_expanded(&area, &heights, ground, chunk, key),
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
    heights: &Heightfield,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
    key: u32,
) -> Expanded {
    let placed = expand::props_in(&area.prop_sources, ground, chunk);
    Expanded {
        drawn: placed
            .iter()
            .map(|placed| Drawn::Prop(placed.prop))
            .collect(),
        records: placed
            .iter()
            .map(|placed| prop_record(heights, placed, key))
            .collect(),
    }
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
    Expanded { records, drawn }
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
/// frame's budget holds, wanting the models of what they draw, each taking the place of what
/// stood at its pair's other key.
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
        let placed = instances
            .arena
            .insert(key, &expanded.records, |prop| models_of[&prop]);
        instances.stale.remove(&key);
        instances.standing.insert(
            key,
            Standing {
                lot,
                drawn: expanded.drawn,
                instances: expanded.records.len() as u32,
            },
        );
        changed(world, root, placed);
        let [first, replacing] = sibling_keys(key);
        let replaced = if key == first { replacing } else { first };
        take_away(world, &mut streaming.instances, root, replaced);
    }
}

/// Lets go of what stands at `key` in the arena.
fn take_away(world: &mut World, instances: &mut Instances, root: Entity, key: u32) {
    instances.stale.remove(&key);
    if instances.standing.remove(&key).is_some() {
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
    let materials: Option<Vec<Handle<WorldMaterial>>> = parts
        .iter()
        .map(|material| {
            let look = material
                .as_ref()
                .and_then(|material| names.get(&material.id()))
                .and_then(|name| looks.iter().find(|look| look.material == *name));
            material::part_material(world, model, material.as_ref(), look)
        })
        .collect();
    Some(DressedProp {
        materials: materials?.into_iter().map(Handle::untyped).collect(),
        shadow: def.shadow,
        fades: def.fade,
    })
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
