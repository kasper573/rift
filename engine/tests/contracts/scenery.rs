use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::animation::AnimationPlugin;
use bevy::asset::AssetPlugin;
use bevy::camera::visibility::{RenderLayers, VisibilityPlugin};
use bevy::gltf::{Gltf, GltfPlugin};
use bevy::light::FogVolume;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use bevy::shader::Shader;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::content::Content;
use engine::core::ground;
use engine::core::math::Pos;
use engine::core::render::camera::Lens;
use engine::core::render::camera::WorldPicture;
use engine::core::render::world::cover::CoverMaterial;
use engine::core::render::world::gpu::draw::{GroundDraw, PATCH_CELLS};
use engine::core::render::world::haze::Haze;
use engine::core::render::world::sky::Daylight;
use engine::core::render::world::{self, WorldScenePlugin};
use engine::core::units::{Degrees, MeterSize, Meters};
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::core::vfx::shader::Stage;
use engine::systems::area::scenery::{Eyes, Sight};
use engine::systems::area::{self, Area, AreaDef, scenery};
use serde_json::json;
use testkit::{Game, Map, path};

/// A field whose turf is strewn with pebbles, `ratio` of the ground under them.
fn field(ratio: f32) -> Game {
    strewn_field(ratio, &Map::open(16, 16), json!([]))
}

/// A game whose field is `map` with `scene` on it, its turf strewn with pebbles at `ratio`.
fn strewn_field(ratio: f32, map: &Map, scene: serde_json::Value) -> Game {
    Game::new()
        .shader_on("meadow", Stage::Cover)
        .prop("pebble", json!({ "type": "none" }))
        .prop("tuft", json!({ "type": "none" }))
        .area("field", map, scene)
        .hero("field", 1.0, 1.0)
        .edit("terrain", testkit::TURF, |row| {
            row["cover"] = json!({
                "shader": path("vfx_shader", "meadow"),
                "seed": 7,
                "pieces": [{
                    "shape": { "type": "prop", "prop": path("prop", "pebble") },
                    "ratio": ratio,
                    "size": { "min": 0.2, "max": 0.4 },
                    "color": { "from": [60, 120, 50, 255], "to": [90, 160, 70, 255] },
                }, {
                    "shape": { "type": "blade", "width": 0.05, "height": 0.3 },
                    "ratio": ratio,
                    "size": { "min": 0.8, "max": 1.2 },
                    "color": { "from": [60, 120, 50, 255], "to": [90, 160, 70, 255] },
                }],
            })
        })
}

/// What a client's world holds, as the counts of every kind of asset scenery is made of, the
/// vertices of all its meshes, and its entities.
#[derive(Debug, PartialEq, Eq)]
struct Held {
    meshes: usize,
    vertices: usize,
    images: usize,
    materials: usize,
    covers: usize,
    models: usize,
    air: usize,
    entities: u32,
}

fn held(app: &App) -> Held {
    let world = app.world();
    let meshes = world.resource::<Assets<Mesh>>();
    Held {
        meshes: meshes.len(),
        vertices: meshes.iter().map(|(_, mesh)| mesh.count_vertices()).sum(),
        images: world.resource::<Assets<Image>>().len(),
        materials: world.resource::<Assets<StandardMaterial>>().len(),
        covers: world.resource::<Assets<CoverMaterial>>().len(),
        models: world.resource::<Assets<Gltf>>().len(),
        air: world.resource::<Assets<ScatteringMedium>>().len(),
        entities: world.entities().count_spawned(),
    }
}

/// A client's world drawing `game`'s field, written to `folder`, its shaders compiled.
fn client(game: Game, folder: &Path) -> (App, Arc<Area>) {
    game.write_to(folder);
    let content = game.load();
    let id = content
        .by_path::<AreaDef>(&path("area", "field"))
        .expect("the field loads");
    let area = area::picture(&content, area::Pictured::Area(id), area::Kept::All)
        .expect("the field is pictured")
        .area;
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: folder.display().to_string(),
            ..Default::default()
        },
        WorldSerializationPlugin,
        AnimationPlugin,
        GltfPlugin::default(),
        bevy::mesh::MeshPlugin,
        TransformPlugin,
        VisibilityPlugin,
    ))
    .init_asset::<Shader>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .init_asset::<ScatteringMedium>()
    .add_plugins((
        bevy::pbr::PbrPlugin::default(),
        VfxMaterialsPlugin,
        WorldScenePlugin,
        scenery::SceneryPlugin,
    ))
    .insert_resource(testkit::look())
    .insert_resource(content);
    app.finish();
    app.cleanup();
    assert!(
        settle(&mut app, |app| {
            let world = app.world();
            scenery::shaders_ready(world, world.resource::<Content>(), &area)
        }),
        "the shaders compile"
    );
    (app, area)
}

fn stand(app: &mut App, area: &Arc<Area>) -> scenery::Scenery {
    let content = app.world().resource::<Content>().clone();
    let stood = scenery::spawn(app.world_mut(), &content, area, RenderLayers::default());
    let bounds = area.size.bounds();
    scenery::view_from(
        app.world_mut(),
        stood,
        overhead(
            bounds.center(),
            Meters(bounds.size.width.hypot(bounds.size.height)),
            10.0,
        ),
    );
    assert!(
        settle(app, |app| {
            !scenery::standing(app.world_mut()) && !world::readying(app.world_mut())
        }),
        "the scenery stands whole"
    );
    stood
}

fn clear(app: &mut App, stood: scenery::Scenery, to: &Held) {
    scenery::despawn(app.world_mut(), stood);
    assert!(
        settle(app, |app| held(app) == *to),
        "clearing the scenery leaves {:?} where {to:?} was",
        held(app)
    );
}

/// Steps the world until `done`, then a few frames more for what follows from it; false when it
/// never is.
fn settle(app: &mut App, done: impl Fn(&mut App) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    app.update();
    while !done(app) {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
        app.update();
    }
    for _ in 0..4 {
        app.update();
    }
    true
}

/// What stands of an area follows the view: its props and the copies its covers strew stand in
/// the scenery's arena chunk by chunk as the view reaches them, counted as missing until they do,
/// and the blades they grow stand within the view's reach; all go as the view leaves them behind.
/// Its ground stands finer near the view than far from it, and the copies the view takes in from
/// afar stand thinned as the view shows them, fewer than near and none too small to show, those
/// standing until what replaces them does, so a long field costs what the view takes in, not
/// what the field holds.
#[test]
fn an_areas_scenery_stands_what_the_view_takes_in_and_lets_go_of_what_it_leaves() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let game = strewn_field(
        0.0016,
        &Map::open(512, 16),
        json!((0..8).map(|index| json!({ "type": "prop", "prop": path("prop", "tuft"), "at": { "x": 488.0 + index as f32, "y": 8.0 } })).collect::<Vec<_>>()),
    );
    let (mut app, area) = client(game, folder.path());
    let content = app.world().resource::<Content>().clone();
    let stood = scenery::spawn(app.world_mut(), &content, &area, RenderLayers::default());
    let reach = Meters(64.0);
    let view = |app: &mut App, sight: Sight| {
        scenery::view_from(app.world_mut(), stood, sight);
        assert!(settle(app, |app| {
            !scenery::standing(app.world_mut()) && !world::readying(app.world_mut())
        }));
        assert_eq!(
            scenery::missing(app.world()),
            0,
            "the view's props all stand"
        );
        (
            scenery::arena_props(app.world(), stood),
            scenery::arena_copies(app.world(), stood),
        )
    };

    let (tufts, pebbles) = view(&mut app, overhead(Pos::new(16.0, 8.0), reach, 10.0));
    let west = held(&app);
    let west_blades = scenery::grass_blades(app.world(), stood);
    assert!(west_blades > 0, "the turf grows its blades round the view");
    let (near, far) = (
        ground_stride_at(&mut app, 16.0),
        ground_stride_at(&mut app, 480.0),
    );
    assert!(
        far > near,
        "the ground stands coarser far from the view: a sample in {far} far off, in {near} near"
    );
    assert_eq!(tufts, 0, "no tuft stands far east of the view");
    assert!(pebbles > 0, "the pebbles strewn round the view stand");

    let (tufts, east_pebbles) = view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 10.0));
    assert_eq!(tufts, 8, "the tufts stand once the view reaches them");
    assert!(
        east_pebbles > 0,
        "the pebbles strewn east stand once the view reaches them"
    );

    let (tufts, back) = view(&mut app, overhead(Pos::new(16.0, 8.0), reach, 10.0));
    assert_eq!(
        (
            tufts,
            back,
            held(&app),
            scenery::grass_blades(app.world(), stood)
        ),
        (0, pebbles, west, west_blades),
        "back west, the tufts, the pebbles and the blades the east grew are gone again"
    );

    // A pebble spans a pixel or two seen from 1500 m through the long lens, so the GPU keeps but
    // some of them there, and less than a pixel from 20 km, so none shows at all.
    let (_, thinned) = view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 1500.0));
    assert!(
        thinned > 0 && thinned < east_pebbles,
        "seen from farther, the same ground keeps fewer of its pebbles: {thinned} of {east_pebbles}"
    );
    let (_, unseen) = view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 20_000.0));
    assert_eq!(unseen, 0, "from high above, the pebbles don't stand");

    view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 1500.0));
    scenery::view_from(
        app.world_mut(),
        stood,
        overhead(Pos::new(496.0, 8.0), reach, 10.0),
    );
    assert!(settle(&mut app, |app| {
        assert!(
            scenery::arena_copies(app.world(), stood) >= thinned,
            "coming near, the thinned pebbles stand until all of them do"
        );
        !scenery::standing(app.world_mut()) && !world::readying(app.world_mut())
    }));
    assert_eq!(
        scenery::arena_copies(app.world(), stood),
        east_pebbles,
        "near again, every pebble stands as it did"
    );
}

/// Every how many samples the ground is drawn `x` metres along the field.
fn ground_stride_at(app: &mut App, x: f32) -> u32 {
    let world = app.world_mut();
    let column = (x * ground::SAMPLES_PER_METER as f32) as u32;
    world
        .query::<&GroundDraw>()
        .iter(world)
        .flat_map(|draw| draw.patches.iter())
        .find(|patch| {
            let (first, _) = patch.first();
            (first..first + PATCH_CELLS * patch.stride()).contains(&column)
        })
        .map(|patch| patch.stride())
        .expect("ground drawn there")
}

/// A sight of `reach` around `at` from an eye `height` metres straight over it, through a lens 720
/// pixels tall long enough that a pebble a centimetre across spans pixels from hundreds of metres
/// off.
fn overhead(at: Pos<Meters>, reach: Meters, height: f32) -> Sight {
    Sight {
        at,
        reach,
        lens: Lens::new(720.0, Degrees(0.1)),
        eyes: Eyes::standing(Vec3::new(at.x, height, at.y), at, Meters(0.0)),
    }
}

#[test]
fn an_areas_scenery_lets_go_of_all_it_held_when_it_goes_and_holds_as_much_each_time_it_stands() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, area) = client(field(0.0016), folder.path());
    let bare = held(&app);
    let first = stand(&mut app, &area);
    let standing = held(&app);
    assert!(
        standing.meshes > bare.meshes && standing.entities > bare.entities,
        "the field stands: {standing:?} over {bare:?}"
    );
    clear(&mut app, first, &bare);
    let again = stand(&mut app, &area);
    assert_eq!(held(&app), standing, "the field stands again as it stood");
    clear(&mut app, again, &bare);
}

/// Lighting an area at a moment that has not changed changes nothing it lit: what follows the
/// light (the sky's map, the haze pass, the grading) reacts only when the light does.
#[test]
fn lighting_the_same_moment_again_changes_nothing() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, area) = client(field(0.0), folder.path());
    let stood = stand(&mut app, &area);
    let camera = app
        .world_mut()
        .spawn((
            Transform::from_xyz(8.0, 10.0, 8.0),
            GlobalTransform::from_xyz(8.0, 10.0, 8.0),
            WorldPicture(testkit::look().picture),
        ))
        .id();
    let weather = area.climate.held_at(0, 0.0);
    let lit = area.clone();
    app.init_resource::<Relit>()
        .add_systems(Update, move |world: &mut World| {
            scenery::light(world, &lit, &weather, Daylight::Hour(15.0), stood, camera);
        })
        .add_systems(PostUpdate, count_relit);
    app.update();
    let first = std::mem::take(&mut *app.world_mut().resource_mut::<Relit>());
    assert!(
        first.haze > 0 && first.ambient > 0 && first.grading > 0,
        "the first lighting lights the camera: {first:?}"
    );
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        *app.world().resource::<Relit>(),
        Relit::default(),
        "lighting the same moment again changes nothing"
    );
}

/// How many times each part of the light changed, frame after frame.
#[derive(Resource, Default, Debug, PartialEq)]
struct Relit {
    haze: usize,
    ambient: usize,
    grading: usize,
    air: usize,
    sun: usize,
}

fn count_relit(
    mut relit: ResMut<Relit>,
    haze: Query<(), Changed<Haze>>,
    ambient: Query<(), Changed<AmbientLight>>,
    grading: Query<(), Changed<ColorGrading>>,
    air: Query<(Ref<FogVolume>, Ref<Transform>)>,
    sun: Query<(), Changed<DirectionalLight>>,
) {
    relit.haze += haze.iter().count();
    relit.ambient += ambient.iter().count();
    relit.grading += grading.iter().count();
    relit.air += air
        .iter()
        .filter(|(volume, placed)| volume.is_changed() || placed.is_changed())
        .count();
    relit.sun += sun.iter().count();
}
