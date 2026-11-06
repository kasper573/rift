//! The props of an area's scenery as the GPU draws them, where the video preferences ask for it:
//! every chunk the sight reaches holds its props in the scenery's arena. A chunk's props are worked
//! out off the frame from the area's prop sources alone once the ground under it has loaded,
//! nearest the sight first, and sent to the GPU within the frame's upload budget. A chunk the
//! sight leaves past what it keeps is let go.

use std::sync::Arc;

use bevy::gltf::{Gltf, GltfMaterial};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::expand::{self, SceneryChunk};
use super::plan::{KEEP_SHARE, Plan};
use super::props;
use super::{Budget, Streaming, distance_to, ground_loaded};
use crate::core::assets;
use crate::core::ground::Heightfield;
use crate::core::math::{Offset, Rect, Size};
use crate::core::render::world::gpu::arena::{
    DressedProp, InstanceArena, InstanceChange, InstanceChanges, InstanceRecord, PropModels,
};
use crate::core::render::world::gpu::upload;
use crate::core::render::world::material::{self, WorldMaterial};
use crate::core::render::world_point;
use crate::core::units::{MeterSize, Meters};
use crate::systems::area::prop::PropId;
use crate::systems::area::{Area, PropPlacement};

/// What a scenery holds of its props in its arena: the chunks standing there, those being worked
/// out, those worked out and waiting for room in a frame's budget, the models of every prop they
/// stand, held so they stay loaded, and the props dressed in their materials.
#[derive(Default)]
pub(super) struct Instances {
    arena: InstanceArena,
    expanding: HashMap<SceneryChunk, Task<Expanded>>,
    expanded: Vec<(SceneryChunk, Expanded)>,
    models: HashMap<PropId, Handle<Gltf>>,
    dressed: HashSet<PropId>,
    /// The props each chunk standing in the arena stands, by its key.
    standing: HashMap<u32, HashSet<PropId>>,
}

impl Instances {
    /// How many props stand in the arena.
    pub(super) fn props(&self) -> u32 {
        self.arena.instances()
    }
}

/// A chunk's props worked out: its instances, and the props they are.
struct Expanded {
    records: Vec<InstanceRecord>,
    props: HashSet<PropId>,
}

/// Stands in the arena every chunk the plan reaches, as far as the frame's budget goes: how many
/// of them don't stand yet.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    plan: &Plan,
    root: Entity,
    budget: &mut Budget,
) -> u32 {
    let ground = streaming.area.size.bounds();
    let at = plan.seen.at();
    let reach = plan.seen.reach.0;
    let around = Rect::new(
        at - Offset::new(reach, reach),
        Size::new(2.0 * reach, 2.0 * reach),
    );
    let mut wanted: Vec<SceneryChunk> = SceneryChunk::within(ground, around)
        .filter(|chunk| distance_to(chunk.bounds(ground), at) <= reach)
        .collect();
    wanted.sort_by(|a, b| {
        let (a, b) = (
            plan.seen.order(a.bounds(ground).center()),
            plan.seen.order(b.bounds(ground).center()),
        );
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    let instances = &mut streaming.instances;
    instances
        .expanding
        .retain(|chunk, task| match check_ready(task) {
            Some(expanded) => {
                instances.expanded.push((*chunk, expanded));
                false
            }
            None => true,
        });
    send(world, streaming, root, ground);
    dress(world, streaming, root);
    let mut missing = 0;
    for chunk in wanted {
        let key = key_of(ground, chunk);
        let instances = &streaming.instances;
        if let Some(props) = instances.standing.get(&key) {
            if !props.iter().all(|prop| instances.dressed.contains(prop)) {
                missing += 1;
            }
            continue;
        }
        missing += 1;
        let waiting = instances.expanding.contains_key(&chunk)
            || instances.expanded.iter().any(|(held, _)| *held == chunk);
        if waiting || instances.expanding.len() >= EXPANDING_AT_ONCE {
            continue;
        }
        if !ground_loaded(streaming, chunk.bounds(ground)) {
            continue;
        }
        let area = streaming.area.clone();
        let task =
            AsyncComputeTaskPool::get().spawn(async move { expanded(&area, ground, chunk, key) });
        streaming.instances.expanding.insert(chunk, task);
    }
    if missing > 0 {
        budget.defer();
    }
    missing
}

/// Lets go of every chunk standing past what the plan keeps, and of any being worked out there.
pub(super) fn clear(world: &mut World, streaming: &mut Streaming, plan: &Plan, root: Entity) {
    let ground = streaming.area.size.bounds();
    let keep = plan.seen.reach.0 * KEEP_SHARE;
    let kept = |chunk: SceneryChunk| distance_to(chunk.bounds(ground), plan.seen.at()) <= keep;
    let instances = &mut streaming.instances;
    instances.expanding.retain(|&chunk, _| kept(chunk));
    instances.expanded.retain(|&(chunk, _)| kept(chunk));
    let gone: Vec<u32> = instances
        .arena
        .chunks()
        .filter(|&key| !kept(chunk_of(ground, key)))
        .collect();
    for key in gone {
        instances.standing.remove(&key);
        if let Some(change) = instances.arena.remove(key)
            && let Some(mut changes) = world.get_resource_mut::<InstanceChanges>()
        {
            changes.0.push((root, change));
        }
    }
}

/// Lets the GPU go of everything the scenery under `root` held in its arena.
pub(super) fn drop_arena(world: &mut World, root: Entity) {
    if let Some(mut changes) = world.get_resource_mut::<InstanceChanges>() {
        changes.0.push((root, InstanceChange::Dropped));
    }
}

/// The most chunks being worked out at once.
const EXPANDING_AT_ONCE: usize = 8;

/// Sends the chunks worked out to the arena, nearest first as they were asked for, as many as the
/// frame's budget holds, wanting the models of the props they stand.
fn send(world: &mut World, streaming: &mut Streaming, root: Entity, ground: Rect<Meters>) {
    let instances = &mut streaming.instances;
    while let Some((_, expanded)) = instances.expanded.first() {
        let bytes = expanded.records.len() * InstanceRecord::BYTES as usize;
        if !upload::spend(world, bytes) {
            return;
        }
        let (chunk, expanded) = instances.expanded.remove(0);
        let mut models_of = HashMap::new();
        for &prop in &expanded.props {
            let model = instances.models.entry(prop).or_insert_with(|| {
                let path = prop.get(&streaming.content).model.path();
                let model = assets::model(world.resource::<AssetServer>(), path);
                if let Some(mut models) = world.get_resource_mut::<PropModels>() {
                    models.want(&model);
                }
                model
            });
            models_of.insert(prop.index() as u32, model.id());
        }
        let key = key_of(ground, chunk);
        instances.standing.insert(key, expanded.props);
        if let Some(change) = instances
            .arena
            .insert(key, &expanded.records, |prop| models_of[&prop])
            && let Some(mut changes) = world.get_resource_mut::<InstanceChanges>()
        {
            changes.0.push((root, change));
        }
    }
}

/// Dresses every prop the arena stands whose model is on the GPU: each part of the model in the
/// world material its glTF material and the prop's looks give it, once every look's shader is
/// compiled.
fn dress(world: &mut World, streaming: &mut Streaming, root: Entity) {
    let instances = &mut streaming.instances;
    let undressed: Vec<(PropId, Handle<Gltf>)> = instances
        .models
        .iter()
        .filter(|(prop, _)| !instances.dressed.contains(*prop))
        .map(|(prop, model)| (*prop, model.clone()))
        .collect();
    for (prop, model) in undressed {
        let Some(parts) = world
            .get_resource::<PropModels>()
            .and_then(|models| models.parts(model.id()))
            .map(<[_]>::to_vec)
        else {
            continue;
        };
        let def = prop.get(&streaming.content);
        let Some(looks) = props::compiled_looks(world, &streaming.content, def) else {
            continue;
        };
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
                    .and_then(|name| looks.0.iter().find(|look| look.material == *name));
                material::part_material(world, &model, material.as_ref(), look)
            })
            .collect();
        let Some(materials) = materials else {
            continue;
        };
        streaming.instances.dressed.insert(prop);
        if let Some(mut changes) = world.get_resource_mut::<InstanceChanges>() {
            changes.0.push((
                root,
                InstanceChange::Dressed {
                    prop: prop.index() as u32,
                    dressed: Arc::new(DressedProp {
                        materials,
                        shadow: def.shadow,
                    }),
                },
            ));
        }
    }
}

/// The props standing in `chunk` of `area`'s `ground` as the arena holds them, each on the ground
/// under it.
fn expanded(area: &Area, ground: Rect<Meters>, chunk: SceneryChunk, key: u32) -> Expanded {
    let placed = expand::props_in(&area.prop_sources, ground, chunk);
    Expanded {
        props: placed.iter().map(|placed| placed.prop).collect(),
        records: placed
            .iter()
            .map(|placed| record(&area.heights, placed, key))
            .collect(),
    }
}

/// The instance of `placed`, standing as [`super::prop_transform`] stands its scene, with a seed
/// drawn from where it stands.
fn record(heights: &Heightfield, placed: &PropPlacement, chunk: u32) -> InstanceRecord {
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

/// The key a chunk of `ground` has in the arena: its place among the chunks, row by row.
fn key_of(ground: Rect<Meters>, chunk: SceneryChunk) -> u32 {
    chunk.row * across(ground) + chunk.column
}

fn chunk_of(ground: Rect<Meters>, key: u32) -> SceneryChunk {
    SceneryChunk {
        column: key % across(ground),
        row: key / across(ground),
    }
}

fn across(ground: Rect<Meters>) -> u32 {
    (ground.size.width / SceneryChunk::SIDE.0).ceil().max(1.0) as u32
}
