use std::any::TypeId;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::animation::AnimationPlugin;
use bevy::asset::AssetPlugin;
use bevy::camera::CameraProjection;
use bevy::camera::visibility::{RenderLayers, VisibilityPlugin, VisibleEntities};
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
use bevy::gltf::{Gltf, GltfPlugin};
use bevy::light::atmosphere::ScatteringMedium;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy::shader::Shader;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::content::Content;
use engine::core::render::world::{self, WorldScenePlugin};
use engine::core::vfx::cover::CoverMaterial;
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::core::vfx::shader::Stage;
use engine::systems::area::{self, Area, AreaDef, scenery};
use serde_json::json;
use testkit::{Game, Map, path};

/// A field whose turf is strewn with pebbles, `ratio` of the ground under them.
fn field(ratio: f32) -> Game {
    Game::new()
        .shader_on("meadow", Stage::Cover)
        .prop("pebble", json!({ "type": "none" }))
        .area("field", &Map::open(16, 16), json!([]))
        .edit("terrain", testkit::TURF, |row| {
            row["cover"] = json!({
                "shader": path("vfx_shader", "meadow"),
                "seed": 7,
                "pieces": [{
                    "shape": { "type": "prop", "prop": path("prop", "pebble") },
                    "ratio": ratio,
                    "size": { "min": 0.2, "max": 0.4 },
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
        TransformPlugin,
        VisibilityPlugin,
    ))
    .init_asset::<Shader>()
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .init_asset::<ScatteringMedium>()
    .init_asset::<SkinnedMeshInverseBindposes>()
    .add_plugins((VfxMaterialsPlugin, WorldScenePlugin, scenery::SceneryPlugin))
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
    for _ in 0..3 {
        app.update();
    }
    true
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
