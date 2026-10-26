use std::collections::BTreeMap;

use bevy::animation::AnimationClip;
use bevy::animation::graph::AnimationGraph;
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldSerializationPlugin};
use engine::core::geometry::{Degrees, Radians};
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER};
use engine::core::math::Pos;
use engine::core::math::Rgba;
use engine::core::render::camera::ground_hit;
use engine::core::render::{CameraRig, capsule_hit, facing_rotation, world_point};
use engine::core::time::PlaybackRate;
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::render::{self, HeldProp};
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::wearing::Wearing;
use engine::systems::actor::{Action, Actor, ModelDef, Pose};
use engine::systems::item::ItemDef;
use serde_json::json;
use testkit::{Game, Map, path};

const SIZE: GroundSize = GroundSize {
    width: 32,
    height: 32,
};
const WINDOW: Vec2 = Vec2::new(1280.0, 720.0);

/// A hill four metres high in the middle of the ground, sloping down a metre every two.
fn hill() -> Heightfield {
    Heightfield::from_fn(SIZE, |column, row| {
        let x = column as f32 / SAMPLES_PER_METER as f32;
        let y = row as f32 / SAMPLES_PER_METER as f32;
        let out = ((x - 16.0).powi(2) + (y - 16.0).powi(2)).sqrt();
        Meters((4.0 - out / 2.0).max(0.0))
    })
}

#[test]
fn the_pointer_ray_lands_where_the_ground_shows() {
    let heights = hill();
    let bounds = SIZE.bounds();
    let spots = [
        (Pos::new(16.0, 16.0), "the top of the hill"),
        (Pos::new(12.0, 19.0), "the slope"),
        (Pos::new(3.0, 3.0), "the flat"),
    ];
    for (turned, tilted) in [(0.0, -25.0), (45.0, 0.0), (135.0, 15.0), (270.0, -10.0)] {
        let mut rig = CameraRig::default();
        rig.look_at(Pos::new(14.0, 14.0), Meters(1.0));
        rig.turn_now(Degrees(turned));
        rig.tilt_now(Degrees(tilted));
        let view = rig.view(WINDOW);
        for (at, name) in spots {
            let shown = view
                .project(world_point(at, heights.height_at(at)))
                .unwrap_or_else(|| {
                    panic!("{name} is in view with the camera turned {turned}° tilted {tilted}°")
                });
            let landed = ground_hit(&heights, bounds, view.ray(shown)).unwrap_or_else(|| {
                panic!("the ray through {name} lands, turned {turned}° tilted {tilted}°")
            });
            assert!(
                landed.distance(at) < Meters(0.05),
                "pointing at {name} turned {turned}° landed {landed:?}, not {at:?}"
            );
        }
    }
}

#[test]
fn pointing_at_a_body_where_it_shows_picks_it() {
    const RADIUS: f32 = 0.4;
    const HEIGHT: f32 = 2.3;
    let heights = hill();
    let at = Pos::new(12.0, 19.0);
    let foot = world_point(at, heights.height_at(at));
    for zoomed in 0..3 {
        let mut rig = CameraRig::default();
        rig.look_at(Pos::new(14.0, 14.0), Meters(1.0));
        for _ in 0..zoomed {
            rig.zoom_in();
        }
        let view = rig.view(WINDOW);
        for (height, part) in [(0.0, "feet"), (HEIGHT / 2.0, "middle"), (HEIGHT, "head")] {
            let shown = view
                .project(foot + Vec3::Y * height)
                .unwrap_or_else(|| panic!("the body's {part} show at zoom {zoomed}"));
            assert!(
                capsule_hit(view.ray(shown), foot, RADIUS, HEIGHT).is_some(),
                "pointing at the body's {part} at zoom {zoomed} picks it"
            );
        }
        let beside = view
            .project(foot + Vec3::X * (RADIUS * 3.0))
            .expect("the ground beside the body shows");
        assert!(
            capsule_hit(view.ray(beside), foot, RADIUS, HEIGHT).is_none(),
            "pointing a metre beside the body at zoom {zoomed} misses it"
        );
    }
}

#[test]
fn a_model_fronts_the_way_its_actor_faces() {
    let front = Vec3::Z;
    for (toward, heading) in [
        ("east", Pos::new(1.0, 0.0)),
        ("south", Pos::new(0.0, 1.0)),
        ("north-west", Pos::new(-1.0, -1.0)),
    ] {
        let facing = Radians::of(heading - Pos::new(0.0, 0.0));
        let turned = facing_rotation(facing) * front;
        let wanted = world_point(heading, Meters(0.0)).normalize();
        assert!(
            turned.abs_diff_eq(wanted, 1e-5),
            "facing {toward} turns the front to {turned}, not {wanted}"
        );
    }
}

#[test]
fn a_held_prop_hangs_from_its_hand_and_leaves_with_it() {
    let content = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .hero("field", 2.0, 2.0)
        .sword()
        .row(
            "stance",
            "bow",
            json!({ "name": "bow", "hands": 2, "hand": "main", "prop": "last" }),
        )
        .item(
            "bow",
            json!({ "kind": { "type": "equipment", "slot": "hand", "stance": path("stance", "bow") } }),
        )
        .load();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        WorldSerializationPlugin,
    ))
    .init_asset::<AnimationGraph>()
    .init_asset::<AnimationClip>()
    .register_type::<Name>()
    .register_type::<Transform>()
    .register_type::<ChildOf>()
    .add_observer(render::adopt_player)
    .insert_resource(content.clone());
    let sword = testkit::Declared::declared::<ItemDef>(&content, "sword").expect("a sword");
    let bow = testkit::Declared::declared::<ItemDef>(&content, "bow").expect("a bow");
    let hero = testkit::Declared::declared::<ModelDef>(&content, "hero").expect("a hero");
    let body = app
        .world_mut()
        .spawn((
            Actor {
                color: Rgba(0xffffffff),
                facing: engine::core::geometry::Radians(0.0),
                action: Action::Idle,
                pose: Pose::Idle,
                model: hero,
                attack_rate: PlaybackRate(1.0),
            },
            Wearing {
                hands: BTreeMap::from([(HandIndex::MAIN, sword)]),
            },
        ))
        .id();

    app.world_mut()
        .run_system_cached(render::dress)
        .expect("dressed");
    let rig = app
        .world_mut()
        .resource_mut::<Assets<WorldAsset>>()
        .add(WorldAsset::new(rig_scene()));
    app.world_mut().entity_mut(body).insert(WorldAssetRoot(rig));
    for _ in 0..3 {
        app.update();
    }
    app.world_mut()
        .run_system_cached(render::hang)
        .expect("hung");

    let hand = joint(&mut app, "hand.r").expect("the rig stands");
    assert_eq!(
        props(&mut app),
        vec![(hand, sword)],
        "the sword hangs from the right hand"
    );

    app.world_mut().entity_mut(body).insert(Wearing {
        hands: BTreeMap::from([(HandIndex::MAIN, bow), (HandIndex::OFF, bow)]),
    });
    app.world_mut()
        .run_system_cached(render::hang)
        .expect("hung");
    let left = joint(&mut app, "hand.l").expect("the rig stands");
    assert_eq!(
        props(&mut app),
        vec![(left, bow)],
        "a two-hander held in the last of its hands hangs from it alone"
    );

    app.world_mut().entity_mut(body).insert(Wearing::default());
    app.world_mut()
        .run_system_cached(render::hang)
        .expect("hung");
    assert_eq!(props(&mut app), Vec::new(), "the empty hand holds nothing");
}

/// The test rig's joints as its glTF scene would stand them: a root, a body and two hands.
fn rig_scene() -> World {
    let mut world = World::new();
    let root = world.spawn((Name::new("model"), Transform::default())).id();
    let body = world
        .spawn((Name::new("body"), Transform::default(), ChildOf(root)))
        .id();
    for hand in ["hand.r", "hand.l"] {
        world.spawn((Name::new(hand), Transform::default(), ChildOf(body)));
    }
    world
}

fn joint(app: &mut App, name: &str) -> Option<Entity> {
    app.world_mut()
        .query::<(Entity, &Name)>()
        .iter(app.world())
        .find(|(_, named)| named.as_str() == name)
        .map(|(entity, _)| entity)
}

/// Every prop hanging from a hand, by the joint it hangs from and the item it shows.
fn props(app: &mut App) -> Vec<(Entity, engine::systems::item::ItemId)> {
    let world = app.world_mut();
    let mut held = Vec::new();
    for (prop, parent) in world
        .query_filtered::<(Entity, &ChildOf), With<HeldProp>>()
        .iter(world)
    {
        held.push((prop, parent.parent()));
    }
    held.into_iter()
        .map(|(prop, joint)| {
            let item = world
                .query::<&render::Puppet>()
                .iter(world)
                .flat_map(|puppet| {
                    puppet
                        .held()
                        .map(|(_, item, at)| (item, at))
                        .collect::<Vec<_>>()
                })
                .find(|(_, at)| *at == prop)
                .map(|(item, _)| item)
                .expect("a prop of a held item");
            (joint, item)
        })
        .collect()
}
