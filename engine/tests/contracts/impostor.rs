use std::path::Path;
use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::window::ExitCondition;
use engine::core::audio::playback::Listener;
use engine::core::content::Content;
use engine::core::jobs::Handoff;
use engine::core::math::{Pos, Rng};
use engine::core::os::Profile;
use engine::core::render::camera::{self, Lens};
use engine::core::render::video::VideoPreferences;
use engine::core::render::world::clouds::CloudDrift;
use engine::core::render::world::gpu::impostor::{
    ATLAS_PIXELS, CELL_PIXELS, CellView, ImpostorStore, ImpostorTexels, cell_of,
};
use engine::core::render::world::sky::Daylight;
use engine::core::render::world::{self, WorldScenePlugin};
use engine::core::units::{Degrees, Meters};
use engine::core::vfx::rain::{RainField, Shelter};
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::core::vfx::shader::Stage;
use engine::systems::area::scenery::{self, Eyes, SceneryPlugin, Sight};
use engine::systems::area::{self, AreaDef};
use serde_json::json;
use testkit::{Game, Map, box_glb, path};

const FIELD: f32 = 32.0;
const SIDE: u32 = 256;
const FOV: Degrees = Degrees(50.0);
const BLOCK: Pos<Meters> = Pos::new(16.0, 16.0);
/// The block is a tall slab, so from the horizon it fills its cell's height and from above it
/// shows a narrow top.
const BLOCK_SIZE: Vec3 = Vec3::new(1.0, 4.0, 2.0);

/// A prop's model gets an impostor once it stands before a camera: an atlas looking at the model
/// from every way round the upper hemisphere, each cell showing the model where it covers the
/// cell and nothing where it doesn't, as the model's own shaders draw it, in white light: the
/// block's glTF is blue, but the game's shader paints it red, so red is what the atlas shows. The atlas is kept
/// under the profile, and the next start of the same game on the same profile reads it from
/// there instead of baking it again.
#[test]
fn a_prop_models_impostor_is_baked_once_and_read_from_the_profile_after() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let profile = tempfile::tempdir().expect("a scratch profile");
    field().write_to(folder.path());

    let baked = stand(field(), folder.path(), profile.path());
    let atlases = atlases_under(profile.path());
    assert_eq!(
        atlases.len(),
        1,
        "one atlas kept for the one model: {atlases:?}"
    );
    let written = std::fs::metadata(&atlases[0])
        .and_then(|meta| meta.modified())
        .expect("the atlas file's time");
    let texels = ImpostorTexels::from_file(&std::fs::read(&atlases[0]).expect("the atlas file"))
        .expect("the atlas file holds an atlas of this build");
    drop(baked);

    let from_above = cell_of(Vec3::Y).floor().as_uvec2();
    let middle = middle_of(&texels, from_above);
    assert!(
        middle[3] == 255 && middle[0] > middle[2] && middle[0] > middle[1],
        "seen from above, the middle of the cell shows the block's top as its shader paints it, red: {middle:?}"
    );
    let corner = corner_of(&texels, from_above);
    assert_eq!(corner[3], 0, "the cell's corner shows nothing: {corner:?}");
    let horizon = CellView::all().next().expect("a first cell");
    let middle = middle_of(&texels, horizon.cell);
    assert!(
        middle[3] == 255 && middle[0] > middle[2],
        "seen from the horizon, the middle of the cell shows the block's side, red: {middle:?}"
    );

    let again = stand(field(), folder.path(), profile.path());
    let atlases = atlases_under(profile.path());
    assert_eq!(atlases.len(), 1, "the same atlas: {atlases:?}");
    let kept = std::fs::metadata(&atlases[0])
        .and_then(|meta| meta.modified())
        .expect("the atlas file's time");
    assert_eq!(
        kept, written,
        "the atlas was read, not baked and written again"
    );
    assert_eq!(
        again.world().resource::<ImpostorStore>().pending(),
        0,
        "every impostor stands"
    );
}

/// A field with one block on it, standing before a camera through the whole render app on
/// `profile` until nothing is readying: the app, kept so what it holds can be asked.
fn stand(game: Game, folder: &Path, profile: &Path) -> App {
    let content = game.load();
    let id = content
        .by_path::<AreaDef>(&path("area", "field"))
        .expect("the field loads");
    let area = area::picture(&content, area::Pictured::Area(id), area::Kept::All)
        .expect("the field is pictured")
        .area;
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(AssetPlugin {
                file_path: folder.display().to_string(),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .insert_resource(Rng::new(testkit::drawn_seed()))
    .insert_resource(Handoff::NextFrame)
    .insert_resource(Profile(profile.to_path_buf()))
    .add_plugins((WorldScenePlugin, VfxMaterialsPlugin, SceneryPlugin))
    .insert_resource(testkit::look())
    .insert_resource(content.clone())
    .insert_resource(VideoPreferences::lowest())
    .insert_resource(CloudDrift::carried_for(0.0))
    .insert_resource(Daylight::Fixed)
    .init_resource::<RainField>()
    .init_resource::<Shelter>()
    .init_resource::<Listener>();
    while app.plugins_state() != PluginsState::Ready {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();
    assert!(
        settle(&mut app, Duration::from_secs(120), |app| {
            let world = app.world();
            scenery::shaders_ready(world, world.resource::<Content>(), &area)
        }),
        "the shaders compile"
    );
    let world = app.world_mut();
    let middle = Pos::new(FIELD / 2.0, FIELD / 2.0);
    let eye = Vec3::new(middle.x, 20.0, middle.y + 10.0);
    let mut image = Image::new_target_texture(SIDE, SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = world.resource_mut::<Assets<Image>>().add(image);
    world.spawn((
        Camera3d::default(),
        RenderTarget::Image(image.into()),
        camera::lens(FOV, SIDE, SIDE),
        camera::picture(testkit::look().picture),
        Transform::from_translation(eye).looking_at(Vec3::new(middle.x, 0.0, middle.y), Vec3::Y),
    ));
    let stood = scenery::spawn(world, &content, &area, RenderLayers::default());
    scenery::view_from(
        world,
        stood,
        Sight {
            at: middle,
            reach: Meters(FIELD),
            lens: Lens::new(SIDE as f32, FOV),
            eyes: Eyes::standing(eye, middle, Meters(0.0)),
        },
    );
    assert!(
        settle(&mut app, Duration::from_secs(240), |app| {
            !scenery::standing(app.world_mut()) && !world::readying(app.world_mut())
        }),
        "the scenery stands whole, its impostors with it"
    );
    assert!(
        settle(&mut app, Duration::from_secs(60), |app| {
            !atlases_under(app.world().resource::<Profile>().0.as_path()).is_empty()
        }),
        "the atlas is kept under the profile"
    );
    app
}

fn field() -> Game {
    let rows: Vec<String> = (0..32).map(|_| ".".repeat(32)).collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    Game::new()
        .shader_of(
            "red",
            Stage::Prop,
            "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(1.0, 0.0, 0.0, 1.0) * vec4<f32>(light(), 1.0);\n}\nfn sway(world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {\n    return world;\n}\n",
        )
        .row("prop_material", "red", json!({ "shader": path("vfx_shader", "red") }))
        .prop_of(
            "block",
            box_glb(BLOCK_SIZE, "paint", [0.1, 0.2, 1.0, 1.0]),
            json!({
                "footprint": { "type": "none" },
                "materials": { "paint": path("prop_material", "red") },
            }),
        )
        .area(
            "field",
            &Map::of(&rows),
            json!([
                { "type": "prop", "prop": path("prop", "block"), "at": { "x": BLOCK.x, "y": BLOCK.y } },
            ]),
        )
        .hero("field", 2.0, 2.0)
}

/// Every atlas file kept under `profile`.
fn atlases_under(profile: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(profile.join("impostors"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// The albedo and coverage at the middle of `cell`.
fn middle_of(texels: &ImpostorTexels, cell: UVec2) -> [u8; 4] {
    albedo_at(texels, cell * CELL_PIXELS + UVec2::splat(CELL_PIXELS / 2))
}

/// The albedo and coverage a pixel in from `cell`'s top left corner.
fn corner_of(texels: &ImpostorTexels, cell: UVec2) -> [u8; 4] {
    albedo_at(texels, cell * CELL_PIXELS + UVec2::ONE)
}

fn albedo_at(texels: &ImpostorTexels, at: UVec2) -> [u8; 4] {
    let index = ((at.y * ATLAS_PIXELS + at.x) * 4) as usize;
    texels.albedo[index..index + 4]
        .try_into()
        .expect("four bytes a pixel")
}

fn settle(app: &mut App, patience: Duration, mut done: impl FnMut(&mut App) -> bool) -> bool {
    let deadline = Instant::now() + patience;
    app.update();
    while !done(app) {
        if Instant::now() > deadline {
            return false;
        }
        app.update();
    }
    true
}
