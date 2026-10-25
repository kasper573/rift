//! The world as the client draws it in three dimensions: the materials, meshes, lights, marks and
//! fades every scene is built from. What stands in the world is the game's to spawn; this module
//! only knows how it is drawn.

pub mod billboard;
pub mod cover;
pub mod decal;
pub mod fade;
pub mod lights;
pub mod sky;
pub mod terrain;
pub mod toon;

use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;

use super::camera;

pub struct WorldScenePlugin;

impl Plugin for WorldScenePlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "dither.wesl");
        bevy::shader::load_shader_library!(app, "toon_shade.wesl");
        bevy::asset::embedded_asset!(app, "toon.wesl");
        bevy::asset::embedded_asset!(app, "toon_prepass.wesl");
        bevy::asset::embedded_asset!(app, "decal.wesl");
        app.add_plugins((
            MaterialPlugin::<toon::ToonMaterial>::default(),
            MaterialPlugin::<decal::DecalMaterial>::default(),
        ))
        .init_resource::<toon::ToonCache>()
        .add_observer(toon::toon_scenes)
        .add_systems(
            Update,
            (lights::flicker, fade::fade_occluders, cover::strew_loaded),
        )
        .add_systems(
            PostUpdate,
            (decal::lay_decals, billboard::face_camera)
                .after(camera::place)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}
