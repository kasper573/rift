use bevy::animation::AnimationPlugin;
use bevy::anti_alias::smaa::Smaa;
use bevy::gltf::GltfPlugin;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{CascadeShadowConfig, VolumetricFog};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::shader::Shader;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::geometry::Degrees;
use engine::core::render::camera;
use engine::core::render::video::{
    AmbientOcclusionQuality, AntiAliasingQuality, ShadowQuality, VideoPreferences,
};
use engine::core::render::world::WorldScenePlugin;
use engine::core::render::world::lights::{self, SUN_LUX};
use engine::core::vfx::render::VfxMaterialsPlugin;
use strum::VariantArray;

/// How a picture and the sun over it are drawn, in the parts a player's video preferences choose.
#[derive(Debug, PartialEq)]
struct Drawn {
    /// How many cascades the sun's shadows fall in, and whether they're soft; none without shadows.
    shadows: Option<(usize, bool)>,
    occlusion: Option<ScreenSpaceAmbientOcclusionQualityLevel>,
    shafts: bool,
    /// The quality whose smoothing the picture is drawn with.
    smoothing: Option<AntiAliasingQuality>,
    bloom: bool,
}

#[test]
fn a_players_video_preferences_draw_the_world_as_picked_from_the_moment_they_are_picked() {
    let cases = [
        (
            "everything off",
            VideoPreferences {
                shadows: ShadowQuality::Off,
                ambient_occlusion: AmbientOcclusionQuality::Off,
                light_shafts: false,
                anti_aliasing: AntiAliasingQuality::Off,
                bloom: false,
            },
            Drawn {
                shadows: None,
                occlusion: None,
                shafts: false,
                smoothing: None,
                bloom: false,
            },
        ),
        (
            "everything low",
            VideoPreferences {
                shadows: ShadowQuality::Low,
                ambient_occlusion: AmbientOcclusionQuality::Low,
                light_shafts: true,
                anti_aliasing: AntiAliasingQuality::Low,
                bloom: true,
            },
            Drawn {
                shadows: Some((1, false)),
                occlusion: Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
                shafts: true,
                smoothing: Some(AntiAliasingQuality::Low),
                bloom: true,
            },
        ),
        (
            "everything at its best again",
            VideoPreferences::default(),
            best(),
        ),
    ];
    let mut app = client();
    let first = stand(&mut app);
    app.update();
    assert_eq!(
        drawn(&mut app, first),
        best(),
        "unpicked, the world is drawn at its best"
    );
    for (picked, preferences, expected) in cases {
        *app.world_mut().resource_mut::<VideoPreferences>() = preferences;
        app.update();
        assert_eq!(
            drawn(&mut app, first),
            expected,
            "{picked}: the world on view"
        );
        let later = stand(&mut app);
        app.update();
        assert_eq!(
            drawn(&mut app, later),
            expected,
            "{picked}: a world drawn afterwards"
        );
    }
}

/// Everything at its highest quality.
fn best() -> Drawn {
    Drawn {
        shadows: Some((2, true)),
        occlusion: Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
        shafts: true,
        smoothing: Some(AntiAliasingQuality::Ultra),
        bloom: true,
    }
}

/// A client's world drawing nothing but its picture and its sun.
fn client() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        WorldSerializationPlugin,
        AnimationPlugin,
        GltfPlugin::default(),
    ))
    .init_asset::<Shader>()
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .init_asset::<ScatteringMedium>()
    .add_plugins((VfxMaterialsPlugin, WorldScenePlugin));
    app.finish();
    app.cleanup();
    app
}

/// A picture of the world and the sun over it, as a client draws them.
fn stand(app: &mut App) -> (Entity, Entity) {
    let world = app.world_mut();
    let picture = world.spawn((Camera3d::default(), camera::picture())).id();
    let sun = world
        .spawn(lights::sun(
            Color::WHITE,
            SUN_LUX,
            Degrees(45.0),
            Degrees(180.0),
        ))
        .id();
    (picture, sun)
}

fn drawn(app: &mut App, (picture, sun): (Entity, Entity)) -> Drawn {
    let world = app.world();
    let picture = world.entity(picture);
    let sun = world.entity(sun);
    let light = sun.get::<DirectionalLight>().expect("a sun");
    let cascades = sun
        .get::<CascadeShadowConfig>()
        .expect("the sun's cascades");
    Drawn {
        shadows: light
            .shadow_maps_enabled
            .then(|| (cascades.bounds.len(), light.soft_shadow_size.is_some())),
        occlusion: picture
            .get::<ScreenSpaceAmbientOcclusion>()
            .map(|occlusion| occlusion.quality_level),
        shafts: picture.contains::<VolumetricFog>(),
        smoothing: picture.get::<Smaa>().map(|smaa| {
            *AntiAliasingQuality::VARIANTS
                .iter()
                .find(|quality| quality.preset() == Some(smaa.preset))
                .expect("a quality of the picture's smoothing")
        }),
        bloom: picture.contains::<Bloom>(),
    }
}
