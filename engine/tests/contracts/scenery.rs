use std::any::TypeId;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::animation::AnimationPlugin;
use bevy::asset::AssetPlugin;
use bevy::camera::CameraProjection;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{RenderLayers, VisibilityPlugin, VisibleEntities};
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
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

/// What stands of an area follows the view: its props stand in the scenery's arena chunk by chunk
/// as the view reaches them, counted as missing until they do, and the copies its covers strew
/// and the blades they grow stand within the view's reach; all go as the view leaves them behind. Its ground stands finer
/// near the view than far from it, and the covers the view takes in from afar stand merged into a
/// few meshes, so a long field costs what the view takes in, not what the field holds.
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
        scenery::arena_props(app.world(), stood)
    };

    let tufts = view(&mut app, overhead(Pos::new(16.0, 8.0), reach, 10.0));
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
    assert_eq!(
        (tufts, pebbles_between(&mut app, 256.0, 480.0)),
        (0, 0),
        "nothing stands far east of the view"
    );

    let tufts = view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 10.0));
    assert_eq!(tufts, 8, "the tufts stand once the view reaches them");
    assert!(
        pebbles_between(&mut app, 432.0, 480.0) > 0,
        "the pebbles strewn east stand once the view reaches them"
    );

    let tufts = view(&mut app, overhead(Pos::new(16.0, 8.0), reach, 10.0));
    assert_eq!(
        (tufts, held(&app), scenery::grass_blades(app.world(), stood)),
        (0, west, west_blades),
        "back west, the tufts, the pebbles and the blades the east grew are gone again"
    );

    // From so high over the east that a pebble spans less than a pixel, none stands.
    view(&mut app, overhead(Pos::new(496.0, 8.0), reach, 20_000.0));
    assert_eq!(
        pebbles_between(&mut app, 432.0, 480.0),
        0,
        "from high above, the pebbles don't stand"
    );

    // From high over the east, reaching over the whole field, the pebbles stand merged: none in
    // the east stands on its own, while meshes over it hold them. They merge wherever the view
    // could turn to see them, so the west, far off the eye, holds them too.
    let tufts = view(
        &mut app,
        overhead(Pos::new(476.0, 8.0), Meters(512.0), 300.0),
    );
    assert_eq!(
        (tufts, pebbles_between(&mut app, 256.0, 512.0)),
        (8, 0),
        "from high over the east, the tufts stand and no pebble east stands on its own"
    );
    assert!(
        merged_over::<CoverMaterial>(&mut app, Vec3::new(460.0, 0.0, 8.0))
            && merged_over::<CoverMaterial>(&mut app, Vec3::new(40.0, 0.0, 8.0)),
        "the pebbles the view could turn to see stand merged"
    );

    scenery::view_from(
        app.world_mut(),
        stood,
        overhead(Pos::new(496.0, 8.0), reach, 10.0),
    );
    assert!(settle(&mut app, |app| {
        assert!(
            merged_over::<CoverMaterial>(app, Vec3::new(460.0, 0.0, 8.0))
                || pebbles_between(app, 448.0, 480.0) > 0,
            "coming near, the merged pebbles stand on until their own copies do"
        );
        !scenery::standing(app.world_mut()) && !world::readying(app.world_mut())
    }));
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

/// Whether a mesh drawn in `M` standing at the world's origin, as merged meshes stand, holds what
/// stands over `at`.
fn merged_over<M: Material>(app: &mut App, at: Vec3) -> bool {
    let world = app.world_mut();
    world
        .query_filtered::<(&Aabb, &Transform), With<MeshMaterial3d<M>>>()
        .iter(world)
        .any(|(bounds, placed)| {
            let (low, high) = (Vec3::from(bounds.min()), Vec3::from(bounds.max()));
            placed.translation == Vec3::ZERO
                && (low.x..=high.x).contains(&at.x)
                && (low.z..=high.z).contains(&at.z)
        })
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

/// How many pebbles stand between `from` and `to` along the field: whatever stands with a place
/// of its own there, as the ground, the covers and their chunks stand at the origin.
fn pebbles_between(app: &mut App, from: f32, to: f32) -> usize {
    let world = app.world_mut();
    world
        .query::<&Transform>()
        .iter(world)
        .filter(|placed| placed.translation.x > from && placed.translation.x < to)
        .count()
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

#[test]
fn however_thickly_a_prop_is_strewn_its_copies_hold_no_more_mesh_than_one() {
    let stood = |ratio: f32| {
        let folder = tempfile::tempdir().expect("a scratch folder");
        let (mut app, area) = client(field(ratio), folder.path());
        stand(&mut app, &area);
        held(&app)
    };
    let (sparse, thick) = (stood(0.0004), stood(0.0016));
    assert_eq!(
        (thick.meshes, thick.vertices),
        (sparse.meshes, sparse.vertices),
        "the copies share their prop's mesh"
    );
}

#[test]
fn a_camera_draws_the_strewn_copies_it_sees_and_passes_over_the_rest() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, area) = client(field(0.0016), folder.path());
    let camera = app.world_mut().spawn(Camera3d::default()).id();
    let (first, second) = (Vec3::new(2.0, 0.0, 2.0), Vec3::new(14.0, 0.0, 14.0));
    look_down_on(&mut app, camera, first);
    stand(&mut app, &area);
    for corner in [first, second, first] {
        look_down_on(&mut app, camera, corner);
        let copies = drawn_copies(&mut app, camera);
        let (near, far): (Vec<_>, Vec<_>) = copies
            .iter()
            .partition(|(at, _)| at.with_y(0.0).distance(corner) < 1.5);
        assert!(
            !near.is_empty() && near.iter().all(|(_, shown)| *shown),
            "every copy under the camera is drawn: {near:?}"
        );
        assert!(
            far.iter()
                .all(|(at, shown)| at.with_y(0.0).distance(corner) < 12.0 || !*shown),
            "copies across the field, out of the camera's view, are passed over"
        );
    }
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

/// Points `camera` straight down at `corner` from a few metres up, and lets the world see it.
fn look_down_on(app: &mut App, camera: Entity, corner: Vec3) {
    let eye = Transform::from_translation(corner + Vec3::Y * 6.0).looking_at(corner, Vec3::Z);
    let lens = PerspectiveProjection {
        fov: 0.5,
        ..Default::default()
    };
    app.world_mut().entity_mut(camera).insert((
        lens.compute_frustum(&GlobalTransform::from(eye)),
        Projection::Perspective(lens),
        eye,
    ));
    for _ in 0..3 {
        app.update();
    }
}

/// Where every strewn copy stands, and whether `camera` draws it.
fn drawn_copies(app: &mut App, camera: Entity) -> Vec<(Vec3, bool)> {
    let world = app.world_mut();
    let drawn: Vec<Entity> = world
        .get::<VisibleEntities>(camera)
        .expect("the camera lists what it draws")
        .iter(TypeId::of::<Mesh3d>())
        .copied()
        .collect();
    world
        .query_filtered::<(Entity, &GlobalTransform), (With<MeshMaterial3d<CoverMaterial>>, Allow<Disabled>)>()
        .iter(world)
        .map(|(copy, placed)| (placed.translation(), drawn.contains(&copy)))
        .collect()
}
