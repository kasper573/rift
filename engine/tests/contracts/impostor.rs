use std::path::Path;
use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
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
/// How far the block's shader stretches it sideways past its mesh.
const OVERHANG: f32 = 2.0;
const BLOCK: Pos<Meters> = Pos::new(16.0, 16.0);
/// The block is a tall slab, so from the horizon it fills its cell's height and from above it
/// shows a narrow top.
const BLOCK_SIZE: Vec3 = Vec3::new(1.0, 4.0, 2.0);
/// How high over the field's middle and how far back from it the eye stands: far enough off for
/// the blocks, framed with their overhang, to fall under the band and draw as their impostors.
const FAR: Vec3 = Vec3::new(0.0, 36.0, 56.0);
/// The eye a third of the way in along the same line, where the blocks draw as their meshes, lit
/// from the same side.
const NEAR: Vec3 = Vec3::new(0.0, 12.0, 18.7);
/// How far round the picture's middle the blocks are looked for, from each eye.
const FAR_WINDOW: u32 = 32;
const NEAR_WINDOW: u32 = 100;
/// A plain block, in its glTF's magenta with no shader of the game's, stands west of the painted
/// one.
const PLAIN: Pos<Meters> = Pos::new(11.0, 16.0);
const PLAIN_SIZE: Vec3 = Vec3::new(2.0, 4.0, 2.0);
/// How far an impostor's brightness may stray from its mesh's, seen from the same side.
const ALIKE: f32 = 0.3;
/// A texel in the upper and in the lower half of the block as a horizon cell frames it, on the
/// cell's middle column: the block spans the middle half of the cell's height.
const UPPER: UVec2 = UVec2::new(CELL_PIXELS / 2, 3 * CELL_PIXELS / 8);
const LOWER: UVec2 = UVec2::new(CELL_PIXELS / 2, 5 * CELL_PIXELS / 8);

/// A prop's model gets an impostor once it stands before a camera: an atlas looking at the model
/// from every way round the upper hemisphere, each cell showing the model where it covers the
/// cell and nothing where it doesn't, the way up it stands, as the model's own shaders draw it,
/// in white light: the block's glTF is one blue, but the game's shader paints its upper half red
/// and its lower half blue, so that is what the atlas shows, red over blue; and the shader
/// stretches the block sideways past its mesh, as far as its material's `overhang` says, so the
/// atlas frames the stretched block and shows red beyond the mesh. The camera, far enough off
/// for the blocks to draw as their impostors, sees the painted one the way up it stands, and
/// each impostor, the painted block's and a plain magenta block's drawn by no shader of the
/// game's, as bright as the camera sees its mesh from the same side up close. The atlases are
/// kept under the profile, and the next start of the same game on the same profile reads them
/// from there instead of baking them again.
#[test]
fn a_prop_models_impostor_is_baked_once_and_read_from_the_profile_after() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let profile = tempfile::tempdir().expect("a scratch profile");
    field().write_to(folder.path());

    let (mut baked, image) = stand(field(), folder.path(), profile.path(), FAR);
    let atlases = atlases_under(profile.path());
    assert_eq!(
        atlases.len(),
        2,
        "an atlas kept for each of the two models: {atlases:?}"
    );
    let written = std::fs::metadata(&atlases[0])
        .and_then(|meta| meta.modified())
        .expect("the atlas file's time");
    let from_above = cell_of(Vec3::Y).floor().as_uvec2();
    let texels = atlases
        .iter()
        .map(|atlas| {
            ImpostorTexels::from_file(&std::fs::read(atlas).expect("the atlas file"))
                .expect("the atlas file holds an atlas of this build")
        })
        .find(|texels| {
            let middle = middle_of(texels, from_above);
            middle[0] > middle[2]
        })
        .expect("the painted block's atlas, red from above");
    let seen = picture(&mut baked, &image);
    drop(baked);

    let middle = middle_of(&texels, from_above);
    assert!(
        middle[3] == 255 && middle[0] > middle[2] && middle[0] > middle[1],
        "seen from above, the middle of the cell shows the block's top as its shader paints it, red: {middle:?}"
    );
    let corner = corner_of(&texels, from_above);
    assert_eq!(corner[3], 0, "the cell's corner shows nothing: {corner:?}");
    let horizon = CellView::all().next().expect("a first cell");
    let upper = albedo_at(&texels, horizon.cell * CELL_PIXELS + UPPER);
    let lower = albedo_at(&texels, horizon.cell * CELL_PIXELS + LOWER);
    assert!(
        upper[3] == 255 && upper[0] > upper[2] && lower[3] == 255 && lower[2] > lower[0],
        "seen from the horizon, the cell shows the block's side the way up it stands, red over \
         blue: {upper:?} over {lower:?}"
    );
    let framed = BLOCK_SIZE.length() / 2.0 + OVERHANG;
    let beyond = (BLOCK_SIZE.length() / 2.0 + 0.1) / framed * (CELL_PIXELS / 2) as f32;
    let beyond = albedo_at(
        &texels,
        horizon.cell * CELL_PIXELS + UPPER + UVec2::new(beyond as u32, 0),
    );
    assert!(
        beyond[3] == 255 && beyond[0] > beyond[2],
        "past the mesh's own sphere the cell still shows the stretched block, red: {beyond:?}"
    );
    let past = past_edge(&texels, horizon.cell);
    assert!(
        past[3] == 0 && past[0] > past[2],
        "the texel just past the block's top shows nothing of it but keeps its red, for the reads \
         that blend across the edge: {past:?}"
    );
    let (red, blue) = (
        pixels_of(&seen, FAR_WINDOW, is_red),
        pixels_of(&seen, FAR_WINDOW, is_blue),
    );
    assert!(
        !red.is_empty() && !blue.is_empty() && mean_row(&red) < mean_row(&blue),
        "the camera sees the block's impostor the way up it stands, red over blue: red on rows \
         {:?}, blue on rows {:?}",
        rows(&red),
        rows(&blue)
    );

    let (mut again, image) = stand(field(), folder.path(), profile.path(), NEAR);
    let atlases = atlases_under(profile.path());
    assert_eq!(atlases.len(), 2, "the same atlases: {atlases:?}");
    let near = picture(&mut again, &image);
    for (what, wanted) in [
        ("the painted block", is_painted as fn([u8; 3]) -> bool),
        ("the plain block", is_magenta),
    ] {
        let far = pixels_of(&seen, FAR_WINDOW, wanted);
        let close = pixels_of(&near, NEAR_WINDOW, wanted);
        assert!(
            !far.is_empty() && !close.is_empty(),
            "{what} shows from both eyes: {} pixels far, {} near",
            far.len(),
            close.len()
        );
        let (far, close) = (mean_luma(&far), mean_luma(&close));
        assert!(
            (far - close).abs() <= ALIKE * close,
            "{what}'s impostor is as bright as its mesh seen from the same side: {far} far \
             against {close} near"
        );
    }
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

/// A field with the two blocks on it, standing before a camera `eye` off the field's middle
/// through the whole render app on `profile` until nothing is readying: the app, kept so what it
/// holds can be asked, and the camera's picture.
fn stand(game: Game, folder: &Path, profile: &Path, eye: Vec3) -> (App, Handle<Image>) {
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
    .init_resource::<Listener>()
    .init_resource::<Read>()
    .add_observer(|read: On<ReadbackComplete>, mut taken: ResMut<Read>| {
        taken.0.push((read.entity, read.data.clone()));
    });
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
    let eye = Vec3::new(middle.x, eye.y, middle.y + eye.z);
    let mut image = Image::new_target_texture(SIDE, SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = world.resource_mut::<Assets<Image>>().add(image);
    world.spawn((
        Camera3d::default(),
        RenderTarget::Image(image.clone().into()),
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
            horizon: None,
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
    (app, image)
}

/// The camera's picture as the world stands now, four bytes a pixel, `SIDE` square.
fn picture(app: &mut App, image: &Handle<Image>) -> Vec<u8> {
    camera::forget_history(app.world_mut());
    let readback = app.world_mut().spawn(Readback::texture(image.clone())).id();
    assert!(
        settle(app, Duration::from_secs(60), |app| {
            app.world()
                .resource::<Read>()
                .0
                .iter()
                .any(|(entity, _)| *entity == readback)
        }),
        "the picture is read back"
    );
    let pixels = app
        .world_mut()
        .resource_mut::<Read>()
        .0
        .iter()
        .find(|(entity, _)| *entity == readback)
        .map(|(_, pixels)| pixels.clone())
        .expect("the picture");
    app.world_mut().despawn(readback);
    assert_eq!(
        pixels.len(),
        (SIDE * SIDE * 4) as usize,
        "a picture of {SIDE}² pixels"
    );
    pixels
}

/// What the camera read back, by the readback that asked for it.
#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

/// The pixels within `window` of the picture's middle that are `wanted`: each one's row and
/// colour.
fn pixels_of(pixels: &[u8], window: u32, wanted: impl Fn([u8; 3]) -> bool) -> Vec<(u32, [u8; 3])> {
    let middle = SIDE / 2;
    (middle - window..middle + window)
        .flat_map(|row| (middle - window..middle + window).map(move |column| (row, column)))
        .filter_map(|(row, column)| {
            let at = ((row * SIDE + column) * 4) as usize;
            let color = [pixels[at], pixels[at + 1], pixels[at + 2]];
            wanted(color).then_some((row, color))
        })
        .collect()
}

fn is_red([r, g, b]: [u8; 3]) -> bool {
    r > b.saturating_add(40) && r > g.saturating_add(40)
}

fn is_blue([r, g, b]: [u8; 3]) -> bool {
    b > r.saturating_add(40) && b > g.saturating_add(40)
}

fn is_painted(color: [u8; 3]) -> bool {
    is_red(color) || is_blue(color)
}

fn is_magenta([r, g, b]: [u8; 3]) -> bool {
    r > g.saturating_add(40) && b > g.saturating_add(40)
}

fn rows(pixels: &[(u32, [u8; 3])]) -> Vec<u32> {
    pixels.iter().map(|(row, _)| *row).collect()
}

fn mean_row(pixels: &[(u32, [u8; 3])]) -> f32 {
    rows(pixels).iter().sum::<u32>() as f32 / pixels.len() as f32
}

fn mean_luma(pixels: &[(u32, [u8; 3])]) -> f32 {
    pixels
        .iter()
        .map(|(_, [r, g, b])| 0.299 * f32::from(*r) + 0.587 * f32::from(*g) + 0.114 * f32::from(*b))
        .sum::<f32>()
        / pixels.len() as f32
}

fn field() -> Game {
    let rows: Vec<String> = (0..32).map(|_| ".".repeat(32)).collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    Game::new()
        .shader_of(
            "red",
            Stage::Prop,
            &format!(
                "fn vfx(uv: vec2<f32>) -> vec4<f32> {{\n    let paint = select(vec4<f32>(0.0, 0.0, 1.0, 1.0), vec4<f32>(1.0, 0.0, 0.0, 1.0), local().y > {:?});\n    return paint * vec4<f32>(light(), 1.0);\n}}\nfn sway(world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {{\n    return world + screen_right() * local().x / {:?} * {OVERHANG:?};\n}}\n",
                BLOCK_SIZE.y / 2.0,
                BLOCK_SIZE.x / 2.0
            ),
        )
        .row(
            "prop_material",
            "red",
            json!({ "shader": path("vfx_shader", "red"), "overhang": OVERHANG }),
        )
        .prop_of(
            "block",
            box_glb(BLOCK_SIZE, "paint", [0.1, 0.2, 1.0, 1.0]),
            json!({
                "footprint": { "type": "none" },
                "materials": { "paint": path("prop_material", "red") },
            }),
        )
        .prop_of(
            "plain",
            box_glb(PLAIN_SIZE, "paint", [1.0, 0.0, 1.0, 1.0]),
            json!({ "footprint": { "type": "none" } }),
        )
        .area(
            "field",
            &Map::of(&rows),
            json!([
                { "type": "prop", "prop": path("prop", "block"), "at": { "x": BLOCK.x, "y": BLOCK.y } },
                { "type": "prop", "prop": path("prop", "plain"), "at": { "x": PLAIN.x, "y": PLAIN.y } },
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

/// The first uncovered texel up the middle column of `cell` from its middle.
fn past_edge(texels: &ImpostorTexels, cell: UVec2) -> [u8; 4] {
    (0..CELL_PIXELS / 2)
        .rev()
        .map(|row| {
            albedo_at(
                texels,
                cell * CELL_PIXELS + UVec2::new(CELL_PIXELS / 2, row),
            )
        })
        .find(|texel| texel[3] == 0)
        .expect("the block ends within its cell")
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
