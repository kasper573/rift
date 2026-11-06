//! The world as the client draws it in three dimensions: the material, meshes, lights, marks and
//! fades every scene is built from. What stands in the world is the game's to spawn; this module
//! only knows how it is drawn.

pub mod billboard;
pub mod body;
pub mod clouds;
pub mod cover;
pub mod decal;
pub mod gpu;
pub mod haze;
pub mod layers;
pub mod lights;
pub mod lod;
pub mod mannequin;
pub mod material;
pub mod paint;
pub mod rig_clip;
mod shape;
pub mod sky;
pub mod terrain;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::app::HierarchyPropagatePlugin;
use bevy::camera::visibility::RenderLayers;
use bevy::light::DirectionalLightShadowMap;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

use super::Look;
use super::camera::{self, CameraRig};
use super::video::VideoPreferences;

/// The width of each cascade of the sun's shadow map in texels.
const SHADOW_MAP: usize = 4096;

pub struct WorldScenePlugin;

impl Plugin for WorldScenePlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "shade.wesl");
        bevy::shader::load_shader_library!(app, "prepass_write.wesl");
        bevy::shader::load_shader_library!(app, "clearing.wesl");
        bevy::asset::embedded_asset!(app, "material.wesl");
        bevy::asset::embedded_asset!(app, "material_prepass.wesl");
        bevy::asset::embedded_asset!(app, "decal.wesl");
        app.register_asset_loader(crate::core::assets::loader::PngLoader);
        let compiling = CompilingPipelines::default();
        app.insert_resource(compiling.clone());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(compiling)
                .add_systems(Render, count_compiling.in_set(RenderSystems::Cleanup));
        }
        app.add_plugins((
            MaterialPlugin::<material::WorldMaterial>::default(),
            MaterialPlugin::<decal::DecalMaterial>::default(),
            HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate),
            body::BodyPlugin,
            gpu::draw::GroundDrawPlugin,
            gpu::upload::UploadPlugin,
            gpu::arena::ModelArenaPlugin,
            gpu::cull::PropCullPlugin,
            gpu::prop_draw::PropDrawPlugin,
            gpu::atlas::TerrainAtlasPlugin,
            haze::HazePlugin,
            layers::JointLayersPlugin,
        ))
        .insert_resource(DirectionalLightShadowMap { size: SHADOW_MAP })
        .init_resource::<VideoPreferences>()
        .init_resource::<material::MaterialCache>()
        .init_resource::<cover::CoverVariants>()
        .init_resource::<cover::CopyMeshes>()
        .add_observer(material::dress_scenes)
        .add_observer(mannequin::start)
        .add_observer(camera::finish_new_picture)
        .add_systems(
            First,
            cloud_look.run_if(resource_exists_and_changed::<Look>),
        )
        .add_systems(
            Update,
            (
                camera::refinish_pictures,
                super::video::show_window.run_if(resource_changed::<VideoPreferences>),
                lights::cast_shadows,
                lights::flicker,
                mannequin::hold,
                cover::strew_loaded,
                terrain::stack_pictures,
                material::forget_dropped,
                cover::forget_dropped,
            ),
        )
        .add_systems(
            PostUpdate,
            cover::cull_strewn_chunks
                .after(bevy::camera::visibility::VisibilitySystems::UpdateFrusta)
                .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            (
                decal::lay_decals.run_if(resource_exists::<super::GroundHeights>),
                billboard::face_camera,
            )
                .run_if(resource_exists::<CameraRig>)
                .after(camera::place)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// The clouds look as the game's look says, once it is known and whenever it changes.
fn cloud_look(world: &mut World) {
    let look = world.resource::<Look>().clouds;
    match world.get_resource_mut::<clouds::CloudSheet>() {
        Some(mut sheet) => sheet.restyle(look),
        None => {
            let sheet = clouds::CloudSheet::new(world, look);
            world.insert_resource(sheet);
            world.init_resource::<clouds::CloudDrift>();
        }
    }
}

/// How many pipelines the GPU is still compiling, as the render world last counted. What draws
/// with one is left out of every frame until it is compiled, the first time any of it is seen.
#[derive(Resource, Clone, Default)]
struct CompilingPipelines(Arc<AtomicUsize>);

fn count_compiling(cache: Res<PipelineCache>, compiling: Res<CompilingPipelines>) {
    compiling
        .0
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

/// Whether anything the world draws is still on its way to standing: a model loading or not yet
/// spawned, a texture a model or a cover draws with still decoding, a ground's pictures, a strewn prop, a
/// body's skin waiting on its rig or its paint, or a pipeline something draws with compiling. A
/// view is shown only once nothing is, so it is first seen whole.
pub fn readying(world: &mut World) -> bool {
    let waiting = world
        .query_filtered::<(), Or<(
            With<cover::Strewing>,
            With<terrain::GroundPictures>,
            With<body::SkinOf>,
        )>>()
        .iter(world)
        .next()
        .is_some();
    let models: Vec<Entity> = world
        .query_filtered::<Entity, With<WorldAssetRoot>>()
        .iter(world)
        .collect();
    let mut textures = drawn_textures::<material::WorldLook>(world);
    textures.extend(drawn_textures::<cover::CoverVfx>(world));
    let compiling = world
        .get_resource::<CompilingPipelines>()
        .is_some_and(|compiling| compiling.0.load(Ordering::Relaxed) > 0);
    let Some(assets) = world.get_resource::<AssetServer>() else {
        return waiting || compiling;
    };
    waiting
        || compiling
        || models.into_iter().any(|model| !model_stands(world, model))
        || textures
            .iter()
            .any(|texture| assets.load_state(texture).is_loading())
}

/// The textures of the standard part of every material of kind `E` a mesh draws with.
fn drawn_textures<E: MaterialExtension>(world: &mut World) -> Vec<Handle<Image>> {
    let mut dressed = world.query::<&MeshMaterial3d<ExtendedMaterial<StandardMaterial, E>>>();
    let Some(materials) = world.get_resource::<Assets<ExtendedMaterial<StandardMaterial, E>>>()
    else {
        return Vec::new();
    };
    dressed
        .iter(world)
        .filter_map(|dressed| materials.get(&dressed.0))
        .flat_map(|material| material::textures(&material.base).cloned())
        .collect()
}

/// Whether the model `entity` was spawned with stands: instanced, or failed to load, so nothing
/// more of it is coming.
pub fn model_stands(world: &World, entity: Entity) -> bool {
    let (Some(root), Some(assets), Some(spawner)) = (
        world.get::<WorldAssetRoot>(entity),
        world.get_resource::<AssetServer>(),
        world.get_resource::<WorldInstanceSpawner>(),
    ) else {
        return true;
    };
    assets.load_state(root.0.id()).is_failed()
        || world
            .get::<WorldInstance>(entity)
            .is_some_and(|instance| spawner.instance_is_ready(**instance))
}

/// The assets of a kind dropped since last asked, so what a cache made from them can go too.
fn dropped<A: Asset>(events: &mut MessageReader<AssetEvent<A>>) -> HashSet<AssetId<A>> {
    events
        .read()
        .filter_map(|event| match *event {
            AssetEvent::Removed { id } => Some(id),
            _ => None,
        })
        .collect()
}
