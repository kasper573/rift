use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
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
use engine::core::render::video::{GrassQuality, ShadowQuality, VideoPreferences};
use engine::core::render::world::clouds::CloudDrift;
use engine::core::render::world::gpu::pick::{Outline, Picking};
use engine::core::render::world::sky::Daylight;
use engine::core::render::world::{self, WorldScenePlugin};
use engine::core::units::{Degrees, Meters};
use engine::core::vfx::rain::{RainField, Shelter};
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::core::vfx::shader::Stage;
use engine::systems::area::scenery::{self, Eyes, SceneryPlugin, Sight};
use engine::systems::area::{self, AreaDef};
use serde_json::json;
use tempfile::TempDir;
use testkit::{Game, Map, box_glb, path};

/// The picture is `SIDE` pixels square, looking down at the middle of a field `FIELD` metres
/// across from `EYE_HEIGHT` metres over it and `EYE_BACK` metres toward the viewer, tilted enough
/// to see the blades stand up, through a lens taking in all of it.
const SIDE: u32 = 256;
const FIELD: f32 = 32.0;
const EYE_HEIGHT: f32 = 40.0;
const EYE_BACK: f32 = 12.0;
const FOV: Degrees = Degrees(50.0);

/// Where the props stand on the bare half of the field, and the pebbles and blades are strewn on
/// the other.
const BLOCK: Pos<Meters> = Pos::new(24.0, 10.0);
const LANTERN: Pos<Meters> = Pos::new(24.0, 22.0);
/// How high up each prop the picture is read.
const PROP_HEIGHT: f32 = 2.0;
/// Where the specks are strewn on the bare half, far enough off the eye to fall under the grain,
/// and how wide each is.
const SPECKS: (Pos<Meters>, Pos<Meters>) = (Pos::new(17.0, 1.0), Pos::new(31.0, 7.0));
const SPECK: f32 = 0.35;
/// How tall the props stand: enough to span more than the impostor band on the picture, so each
/// is drawn by its model in its material rather than by its impostor.
const PROP_TALL: f32 = 12.0;

/// A picture of a scenery shows every prop the view sees, each drawn as its material asks: in
/// the world's own material, through a shader of the game's, and strewn as a cover's copies,
/// among the cover's blades; and the props cast their shadows on the ground. The picture is
/// taken as soon as nothing has been readying for two frames running, with the smoothing started
/// over, as a client lifts its veil, so what is first seen is seen whole.
#[test]
fn a_picture_of_a_scenery_shows_its_props_in_every_material_each_shadowing_the_ground() {
    let mut staged = stage();
    let picture = staged.shoot();

    let [r, g, b] = picture.around(Vec3::new(BLOCK.x, PROP_HEIGHT, BLOCK.y), 1);
    assert!(
        b > r && b > g && b > 40.0,
        "the block shows blue, in the world's material: {r} {g} {b}"
    );
    let [r, g, b] = picture.around(Vec3::new(LANTERN.x, PROP_HEIGHT, LANTERN.y), 1);
    assert!(
        r > g && r > b && r > 40.0,
        "the lantern shows red, as the game's shader paints it: {r} {g} {b}"
    );

    let bare = picture.luminances(Pos::new(18.0, 26.0), Pos::new(22.0, 30.0));
    let rock = median(&bare);
    let darkest = (0..24)
        .map(|step| {
            let angle = step as f32 / 24.0 * std::f32::consts::TAU;
            let at = Vec3::new(
                BLOCK.x + 1.5 * angle.cos(),
                0.01,
                BLOCK.y + 1.5 * angle.sin(),
            );
            luminance(picture.around(at, 1))
        })
        .fold(f32::MAX, f32::min);
    assert!(
        darkest < 0.7 * rock,
        "the block's shadow darkens the ground beside it: {darkest} against the bare rock's {rock}"
    );

    let turf = picture.luminances(Pos::new(2.0, 2.0), Pos::new(14.0, 30.0));
    let turf_at = median(&turf);
    let pebbles = turf
        .iter()
        .filter(|&&luminance| luminance > 1.2 * turf_at)
        .count();
    assert!(
        pebbles > 30,
        "the white pebbles strewn on the turf show over it: {pebbles} bright pixels over {turf_at}"
    );
    let blades = picture
        .colors(Pos::new(2.0, 2.0), Pos::new(14.0, 30.0))
        .filter(|[r, g, b]| *r > 1.2 * g && *b > 1.2 * g && *r > 100.0)
        .count();
    assert!(
        blades > 60,
        "the magenta blades grown on the turf show: {blades} magenta pixels"
    );
}

/// Far enough off the eye that the view thins a crowd of props away, the ground under them
/// shows their colour from above by how much of it they cover, so a wood thinned to nothing
/// leaves no bare ground behind.
#[test]
fn the_ground_under_props_the_view_thins_away_shows_their_colour_from_above() {
    let mut staged = stage();
    let picture = staged.shoot();
    let redness = |[r, g, _]: [f32; 3]| r - g;
    let bare: Vec<f32> = picture
        .colors(Pos::new(18.0, 26.0), Pos::new(22.0, 30.0))
        .map(redness)
        .collect();
    let (from, to) = SPECKS;
    let strewn: Vec<f32> = picture
        .colors(
            Pos::new(from.x + 1.0, from.y + 1.0),
            Pos::new(to.x - 1.0, to.y - 1.0),
        )
        .map(redness)
        .collect();
    let (bare, strewn) = (median(&bare), median(&strewn));
    assert!(
        strewn > bare + 15.0,
        "the ground under the red specks reads redder than the bare rock: {strewn} against {bare}"
    );
}

/// Outlining props singles them out on the picture: every pixel within the outline's width of
/// one, showing anything else, takes the outline's colour.
#[test]
fn an_outlined_prop_is_edged_in_the_outlines_colour() {
    let mut staged = stage();
    let picture = staged.shoot();
    let cyan = |[r, g, b]: [f32; 3]| g > 150.0 && r < 0.9 * g && b > 0.9 * g;
    let bare = picture.counted_near(Vec3::new(BLOCK.x, PROP_HEIGHT, BLOCK.y), 12, cyan);
    assert_eq!(bare, 0, "nothing shows cyan before the block is outlined");
    drop(picture);

    let world = staged.app.world_mut();
    let instances: Arc<[(Entity, u32)]> = scenery::instances_of(world, staged.stood, |_| true)
        .into_iter()
        .map(|instance| (staged.stood.root, instance))
        .collect();
    assert!(!instances.is_empty(), "the field's props stand");
    world.entity_mut(staged.camera).insert(Picking {
        pointer: None,
        outlines: [
            Outline {
                color: LinearRgba::new(0.0, 1.0, 1.0, 1.0),
                instances,
            },
            Outline::default(),
        ],
    });
    let mut drawn = 0;
    assert!(
        settle(&mut staged.app, Duration::from_secs(120), |app| {
            let ready = !world::readying(app.world_mut());
            drawn += u32::from(ready);
            drawn >= 3
        }),
        "the outline's pipeline compiles"
    );
    let picture = staged.shoot();
    let edged = picture.counted_near(Vec3::new(BLOCK.x, PROP_HEIGHT, BLOCK.y), 12, cyan);
    let bluest = picture.bluest_near(Vec3::new(BLOCK.x, PROP_HEIGHT, BLOCK.y), 12);
    assert!(
        edged > 20,
        "the block is edged in cyan: {edged} cyan pixels about it, the bluest {bluest:?}"
    );
    let [r, g, b] = picture.around(Vec3::new(BLOCK.x, PROP_HEIGHT, BLOCK.y), 1);
    assert!(
        b > r && b > g,
        "the block itself still shows blue: {r} {g} {b}"
    );
}

/// The field standing whole before a camera whose picture is read back: the app, kept so what
/// it holds can be asked.
struct Staged {
    app: App,
    camera: Entity,
    image: Handle<Image>,
    stood: scenery::Scenery,
    _profile: TempDir,
}

impl Staged {
    /// Takes the camera's picture as the world stands now.
    fn shoot(&mut self) -> Picture<'_> {
        camera::forget_history(self.app.world_mut());
        let readback = self
            .app
            .world_mut()
            .spawn(Readback::texture(self.image.clone()))
            .id();
        assert!(
            settle(&mut self.app, Duration::from_secs(60), |app| {
                app.world()
                    .resource::<Read>()
                    .0
                    .iter()
                    .any(|(entity, _)| *entity == readback)
            }),
            "the picture is read back"
        );
        let pixels = self
            .app
            .world_mut()
            .resource_mut::<Read>()
            .0
            .iter()
            .find(|(entity, _)| *entity == readback)
            .map(|(_, pixels)| pixels.clone())
            .expect("the picture");
        self.app.world_mut().despawn(readback);
        assert_eq!(
            pixels.len(),
            (SIDE * SIDE * 4) as usize,
            "a picture of {SIDE}² pixels"
        );
        Picture {
            pixels,
            app: &self.app,
            camera: self.camera,
        }
    }
}

fn stage() -> Staged {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let profile = tempfile::tempdir().expect("a scratch profile");
    let game = field();
    game.write_to(folder.path());
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
                file_path: folder.path().display().to_string(),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .insert_resource(Rng::new(testkit::drawn_seed()))
    .insert_resource(Handoff::NextFrame)
    .insert_resource(Profile(profile.path().to_path_buf()))
    .add_plugins((WorldScenePlugin, VfxMaterialsPlugin, SceneryPlugin))
    .insert_resource(testkit::look())
    .insert_resource(content.clone())
    .insert_resource(VideoPreferences {
        shadows: ShadowQuality::Low,
        grass: GrassQuality::High,
        ..VideoPreferences::lowest()
    })
    .insert_resource(CloudDrift::carried_for(0.0))
    .insert_resource(Daylight::Fixed)
    .init_resource::<Read>()
    .init_resource::<RainField>()
    .init_resource::<Shelter>()
    .init_resource::<Listener>()
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
    let mut image = Image::new_target_texture(SIDE, SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = world.resource_mut::<Assets<Image>>().add(image);
    let middle = Pos::new(FIELD / 2.0, FIELD / 2.0);
    let eye = Vec3::new(middle.x, EYE_HEIGHT, middle.y + EYE_BACK);
    let look = testkit::look();
    let camera = world
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            camera::lens(FOV, SIDE, SIDE),
            camera::picture(look.picture),
            Transform::from_translation(eye)
                .looking_at(Vec3::new(middle.x, 0.0, middle.y), Vec3::Y),
        ))
        .id();
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
    let weather = area.climate.held_at(0, 0.0);
    let lit = area.clone();
    app.add_systems(Update, move |world: &mut World| {
        scenery::light(world, &lit, &weather, Daylight::Fixed, stood, camera);
    });
    let mut stood_before = false;
    assert!(
        settle(&mut app, Duration::from_secs(240), |app| {
            let stood = !scenery::standing(app.world_mut()) && !world::readying(app.world_mut());
            let seen_whole = stood && stood_before;
            stood_before = stood;
            seen_whole
        }),
        "the scenery stands whole"
    );
    Staged {
        app,
        camera,
        image,
        stood,
        _profile: profile,
    }
}

/// A field whose western half is turf strewn with white pebbles among magenta blades, and whose
/// eastern half is bare rock, where a blue block stands in the world's own material and a
/// lantern that a shader of the game's paints red.
fn field() -> Game {
    let row = format!("{}{}", ".".repeat(16), "#".repeat(16));
    let rows: Vec<&str> = (0..32).map(|_| row.as_str()).collect();
    let white = [1.0, 1.0, 1.0, 1.0];
    Game::new()
        .shader_of(
            "meadow",
            Stage::Cover,
            "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(albedo().rgb * light(), 1.0);\n}\nfn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {\n    return world;\n}\n",
        )
        .shader_of(
            "lamp",
            Stage::Prop,
            "fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(1.0, 0.0, 0.0, 1.0);\n}\nfn sway(world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {\n    return world;\n}\n",
        )
        .row("prop_material", "glow", json!({ "shader": path("vfx_shader", "lamp") }))
        .prop_of(
            "pebble",
            box_glb(Vec3::splat(1.0), "stone", white),
            json!({ "footprint": { "type": "none" } }),
        )
        .prop_of(
            "block",
            box_glb(Vec3::new(1.0, PROP_TALL, 1.0), "paint", [0.1, 0.2, 1.0, 1.0]),
            json!({ "footprint": { "type": "none" } }),
        )
        .prop_of(
            "lantern",
            box_glb(Vec3::new(1.0, PROP_TALL, 1.0), "glow", white),
            json!({
                "footprint": { "type": "none" },
                "materials": { "glow": path("prop_material", "glow") },
            }),
        )
        .prop_of(
            "speck",
            box_glb(Vec3::splat(SPECK), "paint", [1.0, 0.0, 0.0, 1.0]),
            json!({ "footprint": { "type": "none" } }),
        )
        .area(
            "field",
            &Map::of(&rows),
            json!([
                { "type": "prop", "prop": path("prop", "block"), "at": { "x": BLOCK.x, "y": BLOCK.y } },
                { "type": "prop", "prop": path("prop", "lantern"), "at": { "x": LANTERN.x, "y": LANTERN.y } },
                {
                    "type": "scatter",
                    "props": [path("prop", "speck")],
                    "region": { "x": SPECKS.0.x, "y": SPECKS.0.y, "w": SPECKS.1.x - SPECKS.0.x, "h": SPECKS.1.y - SPECKS.0.y },
                    "per_square_meter": 2.0,
                    "seed": 11,
                },
            ]),
        )
        .hero("field", 2.0, 2.0)
        .edit("terrain", testkit::TURF, |row| {
            row["cover"] = json!({
                "shader": path("vfx_shader", "meadow"),
                "seed": 7,
                "pieces": [{
                    "shape": { "type": "prop", "prop": path("prop", "pebble") },
                    "ratio": 0.05,
                    "size": { "min": 0.3, "max": 0.3 },
                    "color": { "from": [255, 255, 255, 255], "to": [255, 255, 255, 255] },
                }, {
                    "shape": { "type": "blade", "width": 0.5, "height": 1.5 },
                    "ratio": 0.05,
                    "size": { "min": 1.0, "max": 1.0 },
                    "color": { "from": [255, 0, 255, 255], "to": [255, 0, 255, 255] },
                }],
            })
        })
}

/// What the camera read back, by the readback that asked for it.
#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

/// The picture taken, and the world it was taken of, to tell where a point of it shows.
struct Picture<'a> {
    pixels: Vec<u8>,
    app: &'a App,
    camera: Entity,
}

impl Picture<'_> {
    /// How many pixels within `radius` of the one showing `at` are `wanted`.
    fn counted_near(&self, at: Vec3, radius: i32, wanted: impl Fn([f32; 3]) -> bool) -> usize {
        let center = self.pixel(at);
        let mut count = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (x, y) = (center.x as i32 + dx, center.y as i32 + dy);
                if (0..SIDE as i32).contains(&x)
                    && (0..SIDE as i32).contains(&y)
                    && wanted(self.at(x as u32, y as u32))
                {
                    count += 1;
                }
            }
        }
        count
    }

    /// The colour at the pixel showing `at`, averaged `radius` pixels around.
    fn around(&self, at: Vec3, radius: i32) -> [f32; 3] {
        let center = self.pixel(at);
        let mut sum = [0.0; 3];
        let mut count = 0.0f32;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (x, y) = (center.x as i32 + dx, center.y as i32 + dy);
                if (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y) {
                    let color = self.at(x as u32, y as u32);
                    for (sum, color) in sum.iter_mut().zip(color) {
                        *sum += color;
                    }
                    count += 1.0;
                }
            }
        }
        sum.map(|sum| sum / count.max(1.0))
    }

    /// The luminance of every pixel showing the ground between `from` and `to`.
    fn luminances(&self, from: Pos<Meters>, to: Pos<Meters>) -> Vec<f32> {
        self.colors(from, to).map(luminance).collect()
    }

    /// The colour of every pixel showing the ground between `from` and `to`.
    fn colors(&self, from: Pos<Meters>, to: Pos<Meters>) -> impl Iterator<Item = [f32; 3]> + '_ {
        let corners = [
            self.pixel(Vec3::new(from.x, 0.0, from.y)),
            self.pixel(Vec3::new(to.x, 0.0, to.y)),
        ];
        let x0 = corners.iter().map(|at| at.x).fold(f32::MAX, f32::min) as u32;
        let x1 = corners.iter().map(|at| at.x).fold(f32::MIN, f32::max) as u32;
        let y0 = corners.iter().map(|at| at.y).fold(f32::MAX, f32::min) as u32;
        let y1 = corners.iter().map(|at| at.y).fold(f32::MIN, f32::max) as u32;
        (y0..y1).flat_map(move |y| (x0..x1).map(move |x| self.at(x, y)))
    }

    /// The five colours within `radius` of the pixel showing `at` that lean most to cyan.
    fn bluest_near(&self, at: Vec3, radius: i32) -> Vec<[f32; 3]> {
        let center = self.pixel(at);
        let mut colors = Vec::new();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (x, y) = (center.x as i32 + dx, center.y as i32 + dy);
                if (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y) {
                    colors.push(self.at(x as u32, y as u32));
                }
            }
        }
        colors.sort_by(|a, b| (b[1] + b[2] - b[0]).total_cmp(&(a[1] + a[2] - a[0])));
        colors.truncate(5);
        colors
    }

    fn at(&self, x: u32, y: u32) -> [f32; 3] {
        let index = ((y * SIDE + x) * 4) as usize;
        [
            f32::from(self.pixels[index]),
            f32::from(self.pixels[index + 1]),
            f32::from(self.pixels[index + 2]),
        ]
    }

    fn pixel(&self, at: Vec3) -> Vec2 {
        let world = self.app.world();
        let camera = world.entity(self.camera);
        camera
            .get::<Camera>()
            .expect("the picture's camera")
            .world_to_viewport(
                camera
                    .get::<GlobalTransform>()
                    .expect("the camera's transform"),
                at,
            )
            .expect("a point within the picture")
    }
}

fn luminance([r, g, b]: [f32; 3]) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

fn median(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[sorted.len() / 2]
}

/// Updates `app` until `done`, within `patience`; whether it was.
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
