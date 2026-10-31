//! The world as the client draws it in three dimensions: the material, meshes, lights, marks and
//! fades every scene is built from. What stands in the world is the game's to spawn; this module
//! only knows how it is drawn.

pub mod billboard;
pub mod body;
pub mod clouds;
pub mod cover;
pub mod decal;
pub mod fade;
pub mod layers;
pub mod lights;
pub mod mannequin;
pub mod material;
pub mod paint;
pub mod rig_clip;
pub mod sky;
pub mod terrain;

use bevy::app::HierarchyPropagatePlugin;
use bevy::camera::visibility::RenderLayers;
use bevy::light::DirectionalLightShadowMap;
use bevy::pbr::MaterialPlugin;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

use super::camera::{self, CameraRig, FullPicture};
use super::video::VideoPreferences;

/// The width of each cascade of the sun's shadow map in texels.
const SHADOW_MAP: usize = 4096;

pub struct WorldScenePlugin;

impl Plugin for WorldScenePlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "shade.wesl");
        bevy::shader::load_shader_library!(app, "prepass_write.wesl");
        bevy::shader::load_shader_library!(app, "clearing.wesl");
        bevy::shader::load_shader_library!(app, "terrain.wesl");
        bevy::asset::embedded_asset!(app, "fade.wesl");
        bevy::asset::embedded_asset!(app, "fade_prepass.wesl");
        bevy::asset::embedded_asset!(app, "decal.wesl");
        bevy::asset::embedded_asset!(app, "ground.wesl");
        app.add_plugins((
            MaterialPlugin::<material::WorldMaterial>::default(),
            MaterialPlugin::<decal::DecalMaterial>::default(),
            MaterialPlugin::<terrain::GroundMaterial>::default(),
            HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate),
            body::BodyPlugin,
            layers::JointLayersPlugin,
        ))
        .insert_resource(DirectionalLightShadowMap { size: SHADOW_MAP })
        .init_resource::<VideoPreferences>()
        .init_resource::<material::MaterialCache>()
        .init_resource::<cover::TexturedCovers>()
        .init_resource::<cover::CopyMeshes>()
        .init_resource::<clouds::CloudSheet>()
        .init_resource::<decal::DecalMesh>()
        .init_resource::<super::GroundHeights>()
        .add_observer(material::dress_scenes)
        .add_observer(mannequin::start)
        .add_observer(camera::finish_new_picture)
        .add_systems(
            Update,
            (
                camera::refinish_pictures,
                lights::cast_shadows,
                lights::flicker,
                mannequin::hold,
                fade::fade_occluders.run_if(resource_exists::<CameraRig>),
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
            (decal::lay_decals, billboard::face_camera)
                .run_if(resource_exists::<CameraRig>)
                .after(camera::place)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }

    fn finish(&self, app: &mut App) {
        let full = FullPicture::on(app.world().get_resource::<RenderAdapterInfo>());
        app.insert_resource(full);
    }
}

/// Whether anything the world draws is still on its way to standing: a model loading or not yet
/// spawned, a texture a model draws with still decoding, a ground's pictures, a strewn prop, or a
/// body's skin waiting on its rig or its paint. A view is shown only once nothing is, so it is
/// first seen whole.
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
    let models: Vec<_> = world
        .query::<(&WorldAssetRoot, Option<&WorldInstance>)>()
        .iter(world)
        .map(|(root, instance)| (root.0.id(), instance.map(|instance| **instance)))
        .collect();
    let mut dressed = world.query::<&MeshMaterial3d<material::WorldMaterial>>();
    let textures: Vec<Handle<Image>> = match world.get_resource::<Assets<material::WorldMaterial>>()
    {
        Some(materials) => dressed
            .iter(world)
            .filter_map(|dressed| materials.get(&dressed.0))
            .flat_map(|material| material::textures(&material.base).cloned())
            .collect(),
        None => Vec::new(),
    };
    let (Some(assets), Some(spawner)) = (
        world.get_resource::<AssetServer>(),
        world.get_resource::<WorldInstanceSpawner>(),
    ) else {
        return waiting;
    };
    waiting
        || models.into_iter().any(|(model, instance)| {
            !assets.load_state(model).is_failed()
                && instance.is_none_or(|instance| !spawner.instance_is_ready(instance))
        })
        || textures
            .iter()
            .any(|texture| assets.load_state(texture).is_loading())
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
