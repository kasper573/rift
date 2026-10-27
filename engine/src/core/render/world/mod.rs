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
pub mod sky;
pub mod terrain;

use bevy::app::HierarchyPropagatePlugin;
use bevy::camera::visibility::RenderLayers;
use bevy::light::DirectionalLightShadowMap;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;

use super::camera::{self, CameraRig, FullPicture};

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
        app.add_plugins((
            MaterialPlugin::<material::WorldMaterial>::default(),
            MaterialPlugin::<decal::DecalMaterial>::default(),
            HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate),
            body::BodyPlugin,
            layers::JointLayersPlugin,
        ))
        .insert_resource(DirectionalLightShadowMap { size: SHADOW_MAP })
        .init_resource::<material::MaterialCache>()
        .init_resource::<clouds::CloudSheet>()
        .init_resource::<decal::DecalMesh>()
        .init_resource::<super::GroundHeights>()
        .add_observer(material::dress_scenes)
        .add_observer(mannequin::start)
        .add_systems(
            Update,
            (
                camera::finish_pictures,
                lights::flicker,
                mannequin::hold,
                fade::fade_occluders.run_if(resource_exists::<CameraRig>),
                cover::strew_loaded,
            ),
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
