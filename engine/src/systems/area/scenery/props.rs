//! The props of an area's scenery, over the regions it is cut into for the view. A prop in a leaf
//! stands as a scene of its own, which fades when it hides the camera's subject and draws its
//! meshes no finer than the view shows them. The props in every coarser region are merged, a mesh
//! for each material they are drawn in, so a view over a whole area draws a few meshes rather
//! than an entity for every prop.

use std::sync::Arc;

use bevy::ecs::system::{SystemParam, SystemState};
use bevy::gltf::Gltf;
use bevy::light::NotShadowCaster;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};

use super::plan::{KEEP_SHARE, Plan, Planner, Seen};
use super::regions::{self, Cut, Merge, Region, Unstood};
use super::{Budget, Making, Merged, PLACEMENT_CELL, Streaming, Streams, take_away};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::ground::{Heightfield, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rect, Size};
use crate::core::render::camera::Lens;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::gpu::upload;
use crate::core::render::world::lod::{MeshLevels, Simplifier};
use crate::core::render::world::material::{MaterialLook, MaterialLooks};
use crate::core::render::world::merged::{
    self, MergedMesh, MergedModel, MergedPlace, ModelSources,
};
use crate::core::render::world::model_stands;
use crate::core::render::world_point;
use crate::core::spatial::SpatialIndex;
use crate::core::units::Meters;
use crate::core::vfx::render as vfx_render;
use crate::core::vfx::uniform::ParamsUniform;
use crate::systems::area::prop::{PropDef, PropId};
use crate::systems::area::{Area, PropPlacement};

/// The transform a prop placed at `placed` stands with on `heights`.
pub fn prop_transform(heights: &Heightfield, placed: &PropPlacement) -> Transform {
    Transform {
        translation: world_point(placed.at, heights.height_at(placed.at) - placed.sunk),
        rotation: Quat::from_rotation_y(-placed.yaw.radians().0),
        scale: Vec3::splat(placed.scale),
    }
}

/// An area's props as its scenery stands them: the models merged so far, and what stands of them.
#[derive(Default)]
pub(super) struct Props {
    models: HashMap<PropId, Model>,
    scenes: HashMap<usize, Entity>,
    /// What stands of each merged region, waiting first on the props it shows to be gathered,
    /// then on their models to be ready to merge.
    merged: HashMap<Merge, Merged<Making<Shown>, MergedMesh>>,
}

/// Where each of an area's props is filed, and the most each measures across, by its index, where
/// its model says.
pub(super) struct Filed {
    index: SpatialIndex,
    extents: Vec<Option<Meters>>,
}

impl Filed {
    /// The props of `area` standing within `bounds`, each in the one region of a level that holds
    /// where it stands.
    fn within<'a>(
        &'a self,
        area: &'a Area,
        bounds: Rect<Meters>,
    ) -> impl Iterator<Item = usize> + 'a {
        self.index
            .within(bounds)
            .filter(move |&index| regions::holds(bounds, area.props[index].at))
    }

    pub(super) fn of(content: &Content, area: &Area) -> Filed {
        let mut index = SpatialIndex::new(PLACEMENT_CELL);
        index.refile_spots(area.props.iter().map(|placed| placed.at));
        let models = content.built::<GltfInfos>();
        let mut sizes: HashMap<PropId, Option<f32>> = HashMap::default();
        let extents = area
            .props
            .iter()
            .map(|placed| {
                let size = *sizes.entry(placed.prop).or_insert_with(|| {
                    let bounds = models.0.get(&placed.prop.get(content).model.path())?.bounds;
                    Some((bounds.max - bounds.min).max_element())
                });
                Some(Meters(size? * placed.scale))
            })
            .collect();
        Filed { index, extents }
    }
}

/// What a plan stands of the area's props: the props in its leaves that stand as scenes, nearest
/// the view first, those of them in each leaf, and the props in its leaves whose scenes stay
/// where they stand already.
pub(super) struct PropsPlan {
    near: Vec<usize>,
    leaves: Vec<(Region, Vec<usize>)>,
    kept: HashSet<usize>,
}

/// What `cut` stands of the area's props, seen from `seen`.
pub(super) fn plan(planner: &Planner, heights: &Heightfield, seen: &Seen, cut: &Cut) -> PropsPlan {
    let in_sight = |index: &usize, share: f32| !out_of_sight(planner, heights, seen, *index, share);
    let leaves: Vec<(Region, Vec<usize>)> = cut
        .leaves
        .iter()
        .map(|&leaf| {
            let within = planner
                .filed
                .within(&planner.area, planner.regions.bounds(leaf));
            (leaf, within.filter(|index| in_sight(index, 1.0)).collect())
        })
        .collect();
    let mut near: Vec<usize> = leaves
        .iter()
        .flat_map(|(_, props)| props)
        .copied()
        .collect();
    let order = |index: usize| seen.order(planner.area.props[index].at);
    near.sort_by(|&a, &b| {
        let (a, b) = (order(a), order(b));
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    let kept = cut
        .leaves
        .iter()
        .flat_map(|&leaf| {
            planner
                .filed
                .within(&planner.area, planner.regions.bounds(leaf))
        })
        .filter(|index| in_sight(index, KEEP_SHARE))
        .collect();
    PropsPlan { near, leaves, kept }
}

/// Stands what `plan` takes in of the area's props: a scene for each in a leaf, and the merged
/// meshes of every coarser region; where it doesn't stand yet.
pub(super) fn stand(
    world: &mut World,
    streaming: &mut Streaming,
    planner: &Arc<Planner>,
    plan: &Plan,
    root: Entity,
    budget: &mut Budget,
) -> Unstood {
    let mut unstood = Unstood::default();
    stand_scenes(world, streaming, root, &plan.props.near, budget);
    for (leaf, props) in &plan.props.leaves {
        if props.iter().any(|index| {
            streaming
                .props
                .scenes
                .get(index)
                .is_none_or(|&scene| !model_stands(world, scene) || !upload::sent(world, scene))
        }) {
            unstood.add(planner.regions.bounds(*leaf));
        }
    }
    let lens = plan.seen.lens();
    for &merge in &plan.cut.merges {
        if !streaming.props.merged.contains_key(&merge) {
            streaming
                .props
                .merged
                .insert(merge, gather(planner, merge, lens));
        }
    }
    let mut wanted_models: HashSet<PropId> = HashSet::default();
    for merge in &plan.cut.merges {
        if let Some(Merged::Waiting(gathering)) = streaming.props.merged.get_mut(merge)
            && let Some(shown) = gathering.made()
        {
            wanted_models.extend(shown.models.iter().copied());
        }
    }
    prepare_models(world, streaming, wanted_models);
    for &merge in &plan.cut.merges {
        if !stand_merge(world, streaming, root, merge, lens, budget) {
            unstood.add(planner.regions.bounds(merge.region));
        }
    }
    unstood
}

/// Takes down what stood of the props for an earlier plan and stands for none in `plan`, wherever
/// what stands for `plan` does. A merge for an earlier plan that shows nothing yet is given up at
/// once.
pub(super) fn clear(
    world: &mut World,
    streaming: &mut Streaming,
    planner: &Planner,
    plan: &Plan,
    unstood: &Unstood,
) {
    let props = &mut streaming.props;
    let area = &planner.area;
    let gone: Vec<usize> = props
        .scenes
        .keys()
        .filter(|&index| {
            !plan.props.kept.contains(index) && unstood.stands_at(area.props[*index].at)
        })
        .copied()
        .collect();
    for index in gone {
        if let Some(scene) = props.scenes.remove(&index) {
            take_away(world, scene);
        }
    }
    let wanted: HashSet<Merge> = plan.cut.merges.iter().copied().collect();
    let regions = &planner.regions;
    props.merged.retain(|merge, merged| {
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

/// The props a merged region shows, and the models they are drawn with.
struct Shown {
    props: Vec<usize>,
    models: Vec<PropId>,
}

/// A prop's model as merged regions need it: loading, being made ready to merge, or ready.
enum Model {
    Loading(Handle<Gltf>, MaterialLooks),
    Preparing(Task<MergedModel>),
    Ready(Arc<MergedModel>),
}

/// Whether a prop spans less than a pixel from the nearest an eye may come to it, or `share`
/// times the reach and more lies between it and where the view looks.
fn out_of_sight(
    planner: &Planner,
    heights: &Heightfield,
    seen: &Seen,
    index: usize,
    share: f32,
) -> bool {
    let placed = &planner.area.props[index];
    placed.at.distance_to(seen.at()) > seen.reach.0 * share
        || planner.filed.extents[index].is_some_and(|extent| {
            let shown_at = seen.ground_detail(placed.at, heights.height_at(placed.at));
            shown_at.distance().0 > seen.lens().shown_within(extent).0 * share
        })
}

/// Stands a scene for every prop of `near` that has none, in its order, as far as the budget goes.
fn stand_scenes(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    near: &[usize],
    budget: &mut Budget,
) {
    let area = streaming.area.clone();
    let missing: Vec<usize> = near
        .iter()
        .filter(|index| !streaming.props.scenes.contains_key(*index))
        .copied()
        .collect();
    for index in missing {
        let placed = &area.props[index];
        if !super::ground_loaded(streaming, ground_under(placed.at)) {
            budget.defer();
            continue;
        }
        if !budget.take(|budget| &mut budget.props) {
            return;
        }
        let model = placed.prop.get(&streaming.content).model.path();
        let scene = GltfAssetLabel::Scene(0).from_asset(model);
        if world.resource::<AssetServer>().get_path_id(scene).is_none()
            && !budget.take(|budget| &mut budget.fresh)
        {
            return;
        }
        let scene = spawn_prop(world, &streaming.content, &streaming.heights, placed, root);
        streaming.props.scenes.insert(index, scene);
    }
}

/// A region's props as it shows them, gathered off the frame to wait to merge: every one in it
/// that spans a pixel through `lens` at the detail the region shows at.
fn gather(planner: &Arc<Planner>, merge: Merge, lens: Lens) -> Merged<Making<Shown>, MergedMesh> {
    let nearest = merge.detail.distance();
    let bounds = planner.regions.bounds(merge.region);
    let planner = planner.clone();
    Merged::Waiting(Making::Task(AsyncComputeTaskPool::get().spawn(
        async move {
            let (filed, area) = (&planner.filed, &planner.area);
            let props: Vec<usize> = filed
                .within(area, bounds)
                .filter(|&index| {
                    filed.extents[index]
                        .is_none_or(|extent| nearest.0 <= lens.shown_within(extent).0)
                })
                .collect();
            let mut models: Vec<PropId> =
                props.iter().map(|&index| area.props[index].prop).collect();
            models.sort_unstable();
            models.dedup();
            Shown { props, models }
        },
    )))
}

/// Stands the merged meshes of a region, once the models of its props are ready to merge and its
/// meshes are made; whether they stand.
fn stand_merge(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    merge: Merge,
    lens: Lens,
    budget: &mut Budget,
) -> bool {
    let pixels_per_meter = lens.pixels_per_meter(merge.detail.distance());
    let props = &mut streaming.props;
    let Some(merged) = props.merged.get_mut(&merge) else {
        return false;
    };
    if let Some(stands) = merged.rise(world, |world, next| Some(spawn_merged(world, next, root))) {
        return stands;
    }
    let Merged::Waiting(gathering) = merged else {
        return false;
    };
    let Some(shown) = gathering.made() else {
        return false;
    };
    let ready: Option<HashMap<PropId, (Arc<MergedModel>, bool)>> = shown
        .models
        .iter()
        .map(|&prop| match props.models.get(&prop) {
            Some(Model::Ready(model)) => {
                Some((prop, (model.clone(), prop.get(&streaming.content).shadow)))
            }
            _ => None,
        })
        .collect();
    let Some(ready) = ready.filter(|_| budget.stage() && budget.take(|budget| &mut budget.merges))
    else {
        return false;
    };
    let shown = std::mem::take(&mut shown.props);
    let (area, heights) = (streaming.area.clone(), streaming.heights.clone());
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let places: Vec<MergedPlace> = shown
            .iter()
            .filter_map(|&index| {
                let placed = &area.props[index];
                let (model, shadow) = ready.get(&placed.prop)?;
                Some(MergedPlace {
                    model: model.clone(),
                    transform: prop_transform(&heights, placed),
                    shadow: *shadow,
                })
            })
            .collect();
        merged::merge(&places, pixels_per_meter)
    });
    props
        .merged
        .insert(merge, Merged::Merging(Making::Task(task)));
    false
}

/// Sets every model of `wanted` on its way to being ready to merge: loading it, then making it
/// ready off the main thread once it and its materials have loaded.
fn prepare_models(world: &mut World, streaming: &mut Streaming, wanted: HashSet<PropId>) {
    let models = &mut streaming.props.models;
    for prop in wanted {
        if models.contains_key(&prop) {
            continue;
        }
        let def = prop.get(&streaming.content);
        let gltf = assets::model(world.resource::<AssetServer>(), def.model.path());
        models.insert(
            prop,
            Model::Loading(gltf, looks_of(world, &streaming.content, def)),
        );
    }
    let mut sources = None;
    for model in models.values_mut() {
        match model {
            Model::Loading(gltf, looks) => {
                let sources =
                    sources.get_or_insert_with(|| SystemState::<ModelSources>::new(world));
                let parts = sources
                    .get_mut(world)
                    .ok()
                    .and_then(|mut sources| sources.parts(gltf, looks));
                if let Some(parts) = parts {
                    *model = Model::Preparing(
                        AsyncComputeTaskPool::get().spawn(async move { parts.prepare() }),
                    );
                }
            }
            Model::Preparing(task) => {
                if let Some(ready) = check_ready(task) {
                    *model = Model::Ready(Arc::new(ready));
                }
            }
            Model::Ready(_) => {}
        }
    }
}

/// The meshes of a scene the scenery stands, each with the mesh it was spawned with and how many
/// times its size it is drawn.
#[derive(Component)]
struct Leveled(Vec<LeveledMesh>);

struct LeveledMesh {
    mesh: Entity,
    full: Handle<Mesh>,
    scale: f32,
}

/// How a scene hangs together: each entity's parent, children and place under its parent.
#[derive(SystemParam)]
pub(super) struct SceneTree<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    children: Query<'w, 's, &'static Children>,
    transforms: Query<'w, 's, &'static Transform>,
}

/// The sceneries standing, and the scenes they stand that keep stand-ins for their meshes.
#[derive(SystemParam)]
pub(super) struct Standing<'w, 's> {
    streams: Option<Res<'w, Streams>>,
    leveled: Query<'w, 's, &'static Leveled>,
}

/// A scene the scenery stands, each time it has spawned, has its meshes simplified into their
/// stand-ins and draws each at the level the scenery's view takes it in at, once it has them; a
/// mesh it leveled before keeps the mesh it was spawned with as its own.
pub(super) fn level_ready_scene(
    ready: On<WorldInstanceReady>,
    tree: SceneTree,
    standing: Standing,
    mut drawn: Query<&mut Mesh3d>,
    mut simplifier: Simplifier,
    mut commands: Commands,
) {
    let SceneTree {
        parents,
        children,
        transforms,
    } = tree;
    let scene = ready.entity;
    let Some(streaming) = parents.get(scene).ok().and_then(|parent| {
        standing
            .streams
            .as_ref()
            .and_then(|streams| streams.0.get(&parent.parent()))
    }) else {
        return;
    };
    let scale_of = |mesh: Entity| {
        std::iter::successors(Some(mesh), |&at| {
            (at != scene)
                .then(|| parents.get(at).ok().map(ChildOf::parent))
                .flatten()
        })
        .filter_map(|at| transforms.get(at).ok())
        .map(|transform| transform.scale.max_element())
        .product()
    };
    let before = standing.leveled.get(scene).ok();
    let leveled = Leveled(
        children
            .iter_descendants(scene)
            .filter_map(|mesh| {
                let full = before
                    .and_then(|before| before.0.iter().find(|leveled| leveled.mesh == mesh))
                    .map_or_else(
                        || drawn.get(mesh).ok().map(|drawn| drawn.0.clone()),
                        |leveled| Some(leveled.full.clone()),
                    )?;
                simplifier.simplify(&full);
                Some(LeveledMesh {
                    mesh,
                    full,
                    scale: scale_of(mesh),
                })
            })
            .collect(),
    );
    if let (Some(plan), Ok(at)) = (&streaming.plan, transforms.get(scene)) {
        for (mesh, level) in levels_for(simplifier.levels(), &plan.seen, at.translation, &leveled) {
            if let Ok(mut drawn) = drawn.get_mut(mesh) {
                drawn.set_if_neq(Mesh3d(level.clone()));
            }
        }
    }
    commands.entity(scene).insert(leveled);
}

/// Draws every standing prop's meshes at the level `seen` takes them in at.
pub(super) fn level_props(world: &mut World, streaming: &Streaming, seen: &Seen) {
    let levels = world.resource::<MeshLevels>();
    let changes: Vec<(Entity, Handle<Mesh>)> = streaming
        .props
        .scenes
        .values()
        .filter_map(|&prop| Some((world.get::<Leveled>(prop)?, world.get::<Transform>(prop)?)))
        .flat_map(|(leveled, at)| levels_for(levels, seen, at.translation, leveled))
        .filter(|(mesh, level)| {
            world
                .get::<Mesh3d>(*mesh)
                .is_some_and(|drawn| drawn.0 != **level)
        })
        .map(|(mesh, level)| (mesh, level.clone()))
        .collect();
    for (mesh, level) in changes {
        if let Some(mut drawn) = world.get_mut::<Mesh3d>(mesh) {
            drawn.0 = level;
        }
    }
}

/// The level each of a scene's meshes is drawn at, `seen` with the scene at `at`: as fine as the
/// nearest an eye may come to it shows it.
fn levels_for<'a>(
    levels: &'a MeshLevels,
    seen: &Seen,
    at: Vec3,
    leveled: &'a Leveled,
) -> impl Iterator<Item = (Entity, &'a Handle<Mesh>)> {
    let distance = seen.detail(seen.eye_distance(at)).distance();
    let lens = seen.lens();
    leveled.0.iter().map(move |mesh| {
        let level = levels
            .of(&mesh.full)
            .and_then(|levels| levels.through(&lens, distance, mesh.scale));
        (mesh.mesh, level.unwrap_or(&mesh.full))
    })
}

fn spawn_merged(world: &mut World, merged: MergedMesh, root: Entity) -> Entity {
    let MergedMesh {
        material,
        shadow,
        mesh,
        bounds,
    } = merged;
    let mesh = upload::stage(world, mesh);
    let mut merged = world.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        bounds,
        ChildOf(root),
    ));
    if !shadow {
        merged.insert(NotShadowCaster);
    }
    merged.id()
}

/// The materials of `def`'s model the game's shaders draw, each with its parameters, of those
/// whose shaders are compiled.
fn looks_of(world: &World, content: &Content, def: &PropDef) -> MaterialLooks {
    MaterialLooks(looks(world, content, def).flatten().collect())
}

/// The materials of `def`'s model the game's shaders draw, each with its parameters, once every
/// one of their shaders is compiled.
pub(super) fn compiled_looks(
    world: &World,
    content: &Content,
    def: &PropDef,
) -> Option<MaterialLooks> {
    looks(world, content, def)
        .collect::<Option<Vec<_>>>()
        .map(MaterialLooks)
}

fn looks<'a>(
    world: &'a World,
    content: &'a Content,
    def: &'a PropDef,
) -> impl Iterator<Item = Option<MaterialLook>> + 'a {
    def.materials.iter().map(|(name, material)| {
        let material = material.get(content);
        Some(MaterialLook {
            material: name.clone(),
            shader: vfx_render::compiled_shader(world, material.shader)?,
            params: ParamsUniform::pack(material.shader.get(content), |param| {
                material.params.get(param).copied()
            }),
            translucency: material.translucency,
        })
    })
}

fn spawn_prop(
    world: &mut World,
    content: &Content,
    heights: &Heightfield,
    placed: &PropPlacement,
    root: Entity,
) -> Entity {
    let def = placed.prop.get(content);
    let scene = assets::model_scene(world.resource::<AssetServer>(), def.model.path());
    let looks = looks_of(world, content, def);
    let mut prop = world.spawn((
        WorldAssetRoot(scene),
        prop_transform(heights, placed),
        looks,
        ChildOf(root),
    ));
    if def.fade {
        prop.insert(Fades::default());
    }
    if !def.shadow {
        prop.insert(NotShadowCaster);
    }
    prop.id()
}

/// The ground a prop placed at `at` stands on: the samples round it its height is read between.
fn ground_under(at: Pos<Meters>) -> Rect<Meters> {
    let sample = 1.0 / SAMPLES_PER_METER as f32;
    Rect::new(at, Size::new(sample, sample))
}
