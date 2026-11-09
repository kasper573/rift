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
use engine::core::render::camera;
use engine::core::render::video::{AntiAliasingQuality, Quality, ShadowQuality, VideoPreferences};
use engine::core::render::world::WorldScenePlugin;
use engine::core::render::world::lights;
use engine::core::units::{Degrees, Meters};
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::systems::area;
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
    /// How far around the player the props, covers and lights stand.
    reach: Meters,
}

#[test]
fn a_players_video_preferences_draw_the_world_as_picked_from_the_moment_they_are_picked() {
    let preset = |quality| VideoPreferences::default().at(quality);
    let cases = [
        (
            "the lowest preset",
            preset(Quality::Lowest),
            Drawn {
                shadows: None,
                occlusion: None,
                shafts: false,
                smoothing: None,
                bloom: false,
                reach: Meters(widest() * 0.5),
            },
        ),
        (
            "the low preset",
            preset(Quality::Low),
            Drawn {
                shadows: Some((1, false)),
                occlusion: Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
                shafts: false,
                smoothing: Some(AntiAliasingQuality::Low),
                bloom: false,
                reach: Meters(widest() * 0.75),
            },
        ),
        (
            "the medium preset",
            preset(Quality::Medium),
            Drawn {
                shadows: Some((1, false)),
                occlusion: Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
                shafts: false,
                smoothing: Some(AntiAliasingQuality::Medium),
                bloom: true,
                reach: Meters(widest() * 0.75),
            },
        ),
        (
            "the high preset",
            preset(Quality::High),
            Drawn {
                smoothing: Some(AntiAliasingQuality::High),
                ..best()
            },
        ),
        (
            "a part picked by hand",
            VideoPreferences {
                shadows: ShadowQuality::Low,
                ..preset(Quality::Lowest)
            },
            Drawn {
                shadows: Some((1, false)),
                occlusion: None,
                shafts: false,
                smoothing: None,
                bloom: false,
                reach: Meters(widest() * 0.5),
            },
        ),
        ("the max preset again", preset(Quality::Max), best()),
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

#[test]
fn a_preset_picks_every_part_at_once_and_a_part_picked_by_hand_leaves_the_rest_as_the_preset_set_them()
 {
    let window = VideoPreferences {
        fullscreen: true,
        vsync: true,
        ..VideoPreferences::default()
    };
    assert_eq!(
        VideoPreferences::default().quality(),
        Some(Quality::Max),
        "unpicked, every part is at its best"
    );
    for &quality in Quality::VARIANTS {
        let picked = window.at(quality);
        assert_eq!(picked.quality(), Some(quality), "{quality:?} picked");
        assert!(
            picked.fullscreen && picked.vsync,
            "{quality:?} leaves the window as it was"
        );
        let by_hand = VideoPreferences {
            bloom: !picked.bloom,
            ..picked
        };
        assert_eq!(
            VideoPreferences {
                bloom: picked.bloom,
                ..by_hand
            },
            picked,
            "{quality:?}, then the bloom by hand: the rest stands as the preset set it"
        );
        assert_eq!(
            by_hand.quality(),
            None,
            "{quality:?}, then the bloom by hand: at no preset"
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
        reach: Meters(widest()),
    }
}

/// How far around the player the props, covers and lights stand at most: the ground the camera
/// takes in from its widest, and as far again for what the frame's edge fades.
fn widest() -> f32 {
    testkit::look().camera.zooms.wide.0 * 4.0
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
        bevy::mesh::MeshPlugin,
    ))
    .init_asset::<Shader>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .init_asset::<ScatteringMedium>()
    .add_plugins((VfxMaterialsPlugin, WorldScenePlugin))
    .insert_resource(testkit::look());
    app.finish();
    app.cleanup();
    app
}

/// A picture of the world and the sun over it, as a client draws them.
fn stand(app: &mut App) -> (Entity, Entity) {
    let world = app.world_mut();
    let look = testkit::look();
    let picture = world
        .spawn((Camera3d::default(), camera::picture(look.picture)))
        .id();
    let sun = world
        .spawn(lights::sun(
            Color::WHITE,
            look.light.sun_lux,
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
        reach: area::render::view_reach(
            &testkit::look().camera,
            world.resource::<VideoPreferences>(),
        ),
    }
}
