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
pub mod model;
pub mod paint;
pub mod rig_clip;
pub mod sky;
pub mod terrain;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use bevy::app::HierarchyPropagatePlugin;
use bevy::camera::visibility::RenderLayers;
use bevy::light::DirectionalLightShadowMap;
use bevy::light::cluster::GlobalClusterSettings;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache, TextureUsages};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Look;
use super::camera::{self, CameraRig};
use super::video::VideoPreferences;

/// The width of each cascade of the sun's shadow map in texels.
const SHADOW_MAP: usize = 2048;

pub struct WorldScenePlugin;

impl Plugin for WorldScenePlugin {
    /// The lights are clustered on the processor: an area holds few to a tile, which the main
    /// world sorts in no time, where the GPU's clustering costs the render world a pass and its
    /// bindings every frame.
    fn finish(&self, app: &mut App) {
        if let Some(mut settings) = app.world_mut().get_resource_mut::<GlobalClusterSettings>() {
            settings.gpu_clustering = None;
        }
    }

    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "shade.wesl");
        bevy::shader::load_shader_library!(app, "prepass_write.wesl");
        bevy::shader::load_shader_library!(app, "clearing.wesl");
        bevy::asset::embedded_asset!(app, "material.wesl");
        bevy::asset::embedded_asset!(app, "material_prepass.wesl");
        bevy::asset::embedded_asset!(app, "decal.wesl");
        app.register_asset_loader(crate::core::assets::loader::PngLoader);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            let progress = RenderProgress::default();
            render_app.insert_resource(progress.clone()).add_systems(
                Render,
                (
                    begin_progress.in_set(RenderSystems::ExtractCommands),
                    count_progress.in_set(RenderSystems::Cleanup),
                ),
            );
            app.insert_resource(progress);
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
            gpu::grass::GrassPlugin,
            gpu::pick::PickPlugin,
            gpu::impostor::ImpostorPlugin,
            gpu::atlas::TerrainAtlasPlugin,
            haze::HazePlugin,
            layers::JointLayersPlugin,
        ))
        .insert_resource(DirectionalLightShadowMap { size: SHADOW_MAP })
        .init_resource::<VideoPreferences>()
        .init_resource::<material::MaterialCache>()
        .init_resource::<cover::CoverVariants>()
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
                terrain::stack_pictures,
                material::forget_dropped,
                cover::forget_dropped,
            ),
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
        )
        .add_systems(
            PostUpdate,
            (
                let_depth_be_read,
                sky::rest_sky_light
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::asset::AssetEventSystems),
            ),
        );
    }
}

/// Every camera's depth can be read once its prepass is drawn, for the passes that sample it.
fn let_depth_be_read(mut cameras: Query<&mut Camera3d, Changed<Camera3d>>) {
    for mut camera in &mut cameras {
        let usages = TextureUsages::from(camera.depth_texture_usages);
        if !usages.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
        }
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

/// What the render world last counted at the end of a frame: how many pipelines the GPU is still
/// compiling, how many frames it has drawn, and whether the last of them drew everything it was
/// given. A draw is left out of a frame while what it needs is on its way: its pipeline
/// compiling, asked for only as the frame that first draws with it is queued, or its material or
/// bindings not yet prepared.
#[derive(Resource, Clone, Default)]
pub(crate) struct RenderProgress(Arc<Progress>);

#[derive(Default)]
struct Progress {
    compiling: AtomicUsize,
    frames: AtomicU64,
    left_out: AtomicBool,
    whole: AtomicBool,
    missing: AtomicU32,
}

impl RenderProgress {
    /// How many frames the render world has drawn.
    pub(crate) fn drawn(&self) -> u64 {
        self.0.frames.load(Ordering::Relaxed)
    }

    /// How many pipelines the GPU was still compiling at the end of the last frame drawn.
    pub(crate) fn compiling(&self) -> usize {
        self.0.compiling.load(Ordering::Relaxed)
    }

    /// Notes that a draw of this frame was left out for want of something still on its way.
    pub(crate) fn left_out(&self) {
        self.0.left_out.store(true, Ordering::Relaxed);
    }

    /// Notes how many instances the camera views' kernels drew coarser than their size asked, in
    /// the latest frame read back.
    pub(crate) fn missing_drawn(&self, instances: u32) {
        self.0.missing.store(instances, Ordering::Relaxed);
    }

    /// Notes a draw left out for want of a pipeline in `state`, unless it failed for good.
    pub(crate) fn awaiting(&self, state: &CachedPipelineState) {
        if !matches!(state, CachedPipelineState::Err(_)) {
            self.left_out();
        }
    }
}

fn begin_progress(progress: Res<RenderProgress>) {
    progress.0.left_out.store(false, Ordering::Relaxed);
}

fn count_progress(cache: Res<PipelineCache>, progress: Res<RenderProgress>) {
    progress
        .0
        .compiling
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
    progress.0.whole.store(
        !progress.0.left_out.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    progress.0.frames.fetch_add(1, Ordering::Relaxed);
}

/// How many frames the render world has drawn, none without one. A change to what the world
/// draws is drawn, and whatever it draws with asked of the GPU, only once the render world has
/// finished the frame it was extracted into: `DRAWN_FRAMES_AFTER` more than it had drawn when the
/// change was made, since the frame being drawn then may have been extracted before it.
pub fn frames_drawn(world: &World) -> Option<u64> {
    world
        .get_resource::<RenderProgress>()
        .map(|progress| progress.drawn())
}

pub const DRAWN_FRAMES_AFTER: u64 = 2;

/// How many prop instances the camera views drew coarser than their size asked, for want of a
/// model or an impostor still on its way, in the latest frame the kernel's count came back from.
pub fn missing_drawn(world: &World) -> u32 {
    world
        .get_resource::<RenderProgress>()
        .map_or(0, |progress| progress.0.missing.load(Ordering::Relaxed))
}

/// Whether anything the world draws is still on its way to standing. A view is shown only once
/// nothing is, so it is first seen whole.
pub fn readying(world: &mut World) -> bool {
    on_its_way(world).any()
}

/// What the world draws that is still on its way to standing: grounds whose pictures are loading
/// or being stacked, bodies whose skin waits on its rig or its paint, pipelines compiling, a draw
/// the latest frame left out for want of something, impostors baking, models loading or not yet
/// spawned, and textures a model or a cover draws with still decoding.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnItsWay {
    pub pictures: u32,
    pub skins: u32,
    pub compiling: u32,
    pub left_out: bool,
    pub baking: u32,
    pub models: u32,
    pub textures: u32,
}

impl OnItsWay {
    pub fn any(self) -> bool {
        self != OnItsWay::default()
    }
}

impl fmt::Display for OnItsWay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counted = [
            ("pictures", self.pictures),
            ("skins", self.skins),
            ("pipelines compiling", self.compiling),
            ("impostors baking", self.baking),
            ("models", self.models),
            ("textures", self.textures),
        ];
        let mut parts: Vec<String> = counted
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(what, count)| format!("{count} {what}"))
            .collect();
        if self.left_out {
            parts.push("a draw left out".to_owned());
        }
        if parts.is_empty() {
            f.write_str("nothing")
        } else {
            f.write_str(&parts.join(", "))
        }
    }
}

pub fn on_its_way(world: &mut World) -> OnItsWay {
    let pictures = world
        .query_filtered::<(), With<terrain::GroundPictures>>()
        .iter(world)
        .count() as u32;
    let skins = world
        .query_filtered::<(), With<body::SkinOf>>()
        .iter(world)
        .count() as u32;
    let models: Vec<Entity> = world
        .query_filtered::<Entity, With<WorldAssetRoot>>()
        .iter(world)
        .collect();
    let mut textures = drawn_textures::<material::WorldLook>(world);
    textures.extend(drawn_textures::<cover::CoverVfx>(world));
    let (compiling, left_out) =
        world
            .get_resource::<RenderProgress>()
            .map_or((0, false), |progress| {
                (
                    progress.compiling() as u32,
                    !progress.0.whole.load(Ordering::Relaxed),
                )
            });
    let Some(assets) = world.get_resource::<AssetServer>() else {
        return OnItsWay {
            pictures,
            skins,
            compiling,
            left_out,
            ..OnItsWay::default()
        };
    };
    OnItsWay {
        pictures,
        skins,
        compiling,
        left_out,
        baking: world
            .get_resource::<gpu::impostor::ImpostorStore>()
            .map_or(0, |store| store.pending() as u32),
        models: models
            .into_iter()
            .filter(|model| !model_stands(world, *model))
            .count() as u32,
        textures: textures
            .iter()
            .filter(|texture| assets.load_state(*texture).is_loading())
            .count() as u32,
    }
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
