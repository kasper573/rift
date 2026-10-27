use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bevy::animation::AnimationPlugin;
use bevy::asset::AssetPlugin;
use bevy::gltf::GltfPlugin;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::audio::playback::{PlaySfx, SfxDef, SfxId, SfxPlace};
use engine::core::content::gltf::PLACEHOLDER_LIFT;
use engine::core::geometry::{Degrees, Radians};
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER};
use engine::core::math::Pos;
use engine::core::render::camera::ground_hit;
use engine::core::render::world::body::{Body, BodyPlugin};
use engine::core::render::world::layers::JointLayersPlugin;
use engine::core::render::world::material::WorldMaterial;
use engine::core::render::{Animator, CameraRig, capsule_hit, facing_rotation, world_point};
use engine::core::text::Typewriter;
use engine::core::time::PlaybackRate;
use engine::core::units::{MeterPos, Meters, MetersPerSec};
use engine::systems::actor::held::Held;
use engine::systems::actor::render::{self, HeldProp, RenderActor};
use engine::systems::actor::skeleton::GestureName;
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Action, Actor, Appearance, Hurt, ModelDef, Pose};
use engine::systems::area::{self, AreaDef, AreaTag};
use engine::systems::item::ItemDef;
use engine::systems::movement::RenderPosition;
use serde_json::json;
use testkit::{APEX, BODY, CLIP, Game, Map, WALK_PACE, path};

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
    let game = Game::new()
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
            json!({ "kind": { "type": "equipment", "wear": "held", "stance": path("stance", "bow") } }),
        );
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, content) = viewer(game, folder.path());
    let sword = testkit::Declared::declared::<ItemDef>(&content, "sword").expect("a sword");
    let bow = testkit::Declared::declared::<ItemDef>(&content, "bow").expect("a bow");
    let body = spawn_hero(&mut app, &content, Action::Idle);
    app.world_mut().entity_mut(body).insert(Held {
        hands: BTreeMap::from([(HandIndex::MAIN, sword)]),
    });
    app.world_mut()
        .run_system_cached(render::dress)
        .expect("dressed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while joint(&mut app, "hand.r").is_none() && Instant::now() < deadline {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
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

    app.world_mut().entity_mut(body).insert(Held {
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

    app.world_mut().entity_mut(body).insert(Held {
        hands: BTreeMap::from([(HandIndex::MAIN, sword), (HandIndex::OFF, sword)]),
    });
    app.world_mut()
        .run_system_cached(render::hang)
        .expect("hung");
    let mut both = props(&mut app);
    both.sort();
    let mut wanted = vec![(hand, sword), (left, sword)];
    wanted.sort();
    assert_eq!(both, wanted, "a one-hander in each hand hangs from each");

    app.world_mut().entity_mut(body).insert(Held::default());
    app.world_mut()
        .run_system_cached(render::hang)
        .expect("hung");
    assert_eq!(props(&mut app), Vec::new(), "the empty hand holds nothing");
}

#[test]
fn a_body_moves_by_the_clip_its_skeleton_plays_for_what_it_does() {
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .hero("field", 2.0, 2.0);
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, content) = viewer(game, folder.path());
    spawn_hero(&mut app, &content, Action::Idle);
    app.world_mut()
        .run_system_cached(render::dress)
        .expect("dressed");

    let held = testkit::rig().nodes[1].rest.translation.y + PLACEHOLDER_LIFT.0;
    let mut shown = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        app.update();
        app.world_mut()
            .run_system_cached(render::pose)
            .expect("posed");
        shown = joint(&mut app, "body").and_then(|body| {
            app.world()
                .get::<Transform>(body)
                .map(|at| at.translation.y)
        });
        if shown.is_some_and(|y| (y - held).abs() < 1e-3) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        shown.map(|y| (y * 1000.0).round() / 1000.0),
        Some(held),
        "the body stands where its idle clip, cut from the skeleton's library, holds it"
    );
}

#[test]
fn a_hand_closes_its_fingers_as_its_clasp_poses_them_while_it_holds_an_item() {
    let closed = Quat::from_rotation_z(1.0);
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .hero("field", 2.0, 2.0)
        .sword()
        .edit("skeleton", BODY, |skeleton| {
            skeleton["hands"][0]["clasp"] = json!({ "clip": "clasp", "fingers": ["finger.r"] })
        })
        .part(
            "skeleton",
            BODY,
            "clips/hands.glb",
            testkit::pose_glb(&testkit::rig(), "clasp", &[("finger.r", closed)]),
        );
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, content) = viewer(game, folder.path());
    let sword = testkit::Declared::declared::<ItemDef>(&content, "sword").expect("a sword");
    let body = spawn_hero(&mut app, &content, Action::Idle);
    app.world_mut()
        .run_system_cached(render::dress)
        .expect("dressed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while joint(&mut app, "finger.r").is_none() && Instant::now() < deadline {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    let finger = joint(&mut app, "finger.r").expect("the rig stands");
    let turned_to = |app: &mut App, held: Held, wanted: Quat| {
        app.world_mut().entity_mut(body).insert(held);
        app.world_mut()
            .run_system_cached(render::hang)
            .expect("hung");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            app.world_mut()
                .run_system_cached(render::pose)
                .expect("posed");
            app.update();
            let shown = app
                .world()
                .get::<Transform>(finger)
                .map(|at| at.rotation)
                .expect("a finger");
            if shown.angle_between(wanted) < 0.01 || Instant::now() > deadline {
                return shown;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };

    let holding = turned_to(
        &mut app,
        Held {
            hands: BTreeMap::from([(HandIndex::MAIN, sword)]),
        },
        closed,
    );
    assert!(
        holding.angle_between(closed) < 0.01,
        "holding a sword, the finger closes as the clasp poses it: {holding:?}"
    );
    let empty = turned_to(&mut app, Held::default(), Quat::IDENTITY);
    assert!(
        empty.angle_between(Quat::IDENTITY) < 0.01,
        "the empty hand opens again: {empty:?}"
    );
}

/// A client's world drawing `game`, written to `folder`: its bodies stand their rigs and skins,
/// and play their skeletons' clips and layers.
fn viewer(game: Game, folder: &std::path::Path) -> (App, engine::core::content::Content) {
    game.write_to(folder);
    let content = game.load();
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
        BodyPlugin,
        JointLayersPlugin,
    ))
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    .init_asset::<WorldMaterial>()
    .init_asset::<Image>()
    .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
    .init_resource::<Animator>()
    .add_observer(render::adopt_player)
    .insert_resource(content.clone());
    app.finish();
    app.cleanup();
    (app, content)
}

/// The declared hero, standing at the middle of the field doing `action`.
fn spawn_hero(app: &mut App, content: &engine::core::content::Content, action: Action) -> Entity {
    let hero = testkit::Declared::declared::<ModelDef>(content, "hero").expect("a hero");
    app.world_mut()
        .spawn((
            Actor {
                facing: Radians(0.0),
                action,
                pose: Pose::Idle,
                attack_rate: PlaybackRate(1.0),
                speed: MetersPerSec::default(),
            },
            Appearance::of(hero),
            RenderActor {
                facing: Radians(0.0),
                action,
            },
            RenderPosition(Pos::new(2.0, 2.0)),
        ))
        .id()
}

#[test]
fn a_body_sounds_the_cues_its_clip_crosses_and_steps_on_the_ground_under_it() {
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("whoosh")
        .skeleton_of(
            BODY,
            &testkit::rig_clips(),
            json!({ "cues": {
                "walk": [{ "at": 100.0, "step": true }],
                "attack": [{ "at": APEX.0, "apex": true, "sound": path("sfx", "whoosh") }],
            } }),
        )
        .hero("field", 2.0, 2.0);
    let content = game.load();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )))
        .init_resource::<Animator>()
        .add_message::<PlaySfx>()
        .insert_resource(content.clone());
    let hero = testkit::Declared::declared::<ModelDef>(&content, "hero").expect("a hero");
    let field = testkit::Declared::declared::<AreaDef>(&content, "field").expect("a field");
    let walker = Pos::new(1.5, 1.5);
    let hurrier = Pos::new(1.5, 3.5);
    let striker = Pos::new(3.5, 3.5);
    for (at, action, speed) in [
        (walker, Action::Walk, WALK_PACE),
        (hurrier, Action::Walk, WALK_PACE * 2.0),
        (striker, Action::Attack, MetersPerSec::default()),
    ] {
        app.world_mut().spawn((
            Actor {
                facing: Radians(0.0),
                action,
                pose: Pose::Idle,
                attack_rate: PlaybackRate(1.0),
                speed,
            },
            Appearance::of(hero),
            RenderActor {
                facing: Radians(0.0),
                action,
            },
            RenderPosition(at),
            AreaTag { area: field },
        ));
    }

    let mut heard: Vec<(Pos<Meters>, SfxId)> = Vec::new();
    for _ in 0..12 {
        app.update();
        app.world_mut()
            .run_system_cached(render::pose)
            .expect("posed");
        heard.extend(
            app.world_mut()
                .resource_mut::<Messages<PlaySfx>>()
                .drain()
                .map(|sound| match sound.place {
                    SfxPlace::World(at) => (at, sound.id),
                    SfxPlace::Interface => panic!("a body sounds where it stands"),
                }),
        );
    }
    let sfx = |name: &str| {
        content
            .by_path::<SfxDef>(&path("sfx", name))
            .unwrap_or_else(|| panic!("the {name} sound"))
    };
    let footstep = area::load(&content, field)
        .footstep_at(walker)
        .copied()
        .expect("the field's ground has a footstep");
    let of = |at: Pos<Meters>| -> Vec<SfxId> {
        heard
            .iter()
            .filter(|(from, _)| *from == at)
            .map(|(_, id)| *id)
            .collect()
    };
    assert!(
        of(walker).len() >= 2 && of(walker).iter().all(|id| *id == footstep),
        "a walker's feet come down once a loop, each sounding the ground under it: {heard:?}"
    );
    assert_eq!(
        of(hurrier).len(),
        2 * of(walker).len(),
        "a walker covering twice the ground its walk covers puts its feet down twice as often"
    );
    assert_eq!(
        of(striker),
        vec![sfx("whoosh")],
        "a strike sounds its apex once, and puts no foot down"
    );
}

#[test]
fn a_looping_clips_variants_take_turns_each_as_the_one_before_plays_through() {
    let clips: Vec<(&str, engine::core::time::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("hum", CLIP.seconds()), ("tap", CLIP.seconds())])
        .collect();
    let mut base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| (action.as_ref().to_owned(), json!(action.as_ref())))
            .collect();
    base.insert(
        "idle".to_owned(),
        json!({ "variants": ["hum", "tap"], "order": "in_turn" }),
    );
    let cue = |sound: &str| json!([{ "at": 100.0, "sound": path("sfx", sound) }]);
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("hum")
        .sfx("tap")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base },
                "cues": { "attack": [{ "at": APEX.0, "apex": true }], "hum": cue("hum"), "tap": cue("tap") },
            }),
        )
        .hero("field", 2.0, 2.0);
    let content = game.load();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )))
        .init_resource::<Animator>()
        .add_message::<PlaySfx>()
        .insert_resource(content.clone());
    let hero = testkit::Declared::declared::<ModelDef>(&content, "hero").expect("a hero");
    let field = testkit::Declared::declared::<AreaDef>(&content, "field").expect("a field");
    app.world_mut().spawn((
        Actor {
            facing: Radians(0.0),
            action: Action::Idle,
            pose: Pose::Idle,
            attack_rate: PlaybackRate(1.0),
            speed: MetersPerSec::default(),
        },
        Appearance::of(hero),
        RenderActor {
            facing: Radians(0.0),
            action: Action::Idle,
        },
        RenderPosition(Pos::new(2.0, 2.0)),
        AreaTag { area: field },
    ));

    let mut heard = Vec::new();
    for _ in 0..30 {
        app.update();
        app.world_mut()
            .run_system_cached(render::pose)
            .expect("posed");
        heard.extend(
            app.world_mut()
                .resource_mut::<Messages<PlaySfx>>()
                .drain()
                .map(|sound| sound.id),
        );
    }
    let sfx = |name: &str| {
        content
            .by_path::<SfxDef>(&path("sfx", name))
            .unwrap_or_else(|| panic!("the {name} sound"))
    };
    let (hum, tap) = (sfx("hum"), sfx("tap"));
    assert!(heard.len() >= 4, "a turn a loop: {heard:?}");
    assert!(
        heard
            .iter()
            .enumerate()
            .all(|(turn, id)| *id == if turn % 2 == 0 { hum } else { tap }),
        "the idle hums, then taps, then hums again: {heard:?}"
    );
}

#[test]
fn a_standing_body_flinches_once_at_each_new_blow() {
    let clips: Vec<(&str, engine::core::time::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("flinch", CLIP.seconds())])
        .collect();
    let mut base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .filter(|action| **action != Action::Hurt)
            .map(|action| (action.as_ref().to_owned(), json!(action.as_ref())))
            .collect();
    base.insert("hurt".to_owned(), json!("flinch"));
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("ow")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base },
                "cues": {
                    "attack": [{ "at": APEX.0, "apex": true }],
                    "flinch": [{ "at": 50.0, "sound": path("sfx", "ow") }],
                },
            }),
        )
        .hero("field", 2.0, 2.0);
    let content = game.load();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )))
        .init_resource::<Animator>()
        .add_message::<PlaySfx>()
        .insert_resource(content.clone());
    let hero = testkit::Declared::declared::<ModelDef>(&content, "hero").expect("a hero");
    let field = testkit::Declared::declared::<AreaDef>(&content, "field").expect("a field");
    let mut body = |at: Pos<Meters>, blows: u32| {
        app.world_mut()
            .spawn((
                Actor {
                    facing: Radians(0.0),
                    action: Action::Idle,
                    pose: Pose::Idle,
                    attack_rate: PlaybackRate(1.0),
                    speed: MetersPerSec::default(),
                },
                Appearance::of(hero),
                RenderActor {
                    facing: Radians(0.0),
                    action: Action::Idle,
                },
                RenderPosition(at),
                AreaTag { area: field },
                Hurt(blows),
            ))
            .id()
    };
    let struck = body(Pos::new(1.5, 1.5), 0);
    let arrived = body(Pos::new(3.5, 3.5), 3);
    let heard = |app: &mut App, frames: usize| {
        let mut heard = Vec::new();
        for _ in 0..frames {
            app.update();
            app.world_mut()
                .run_system_cached(render::pose)
                .expect("posed");
            heard.extend(
                app.world_mut()
                    .resource_mut::<Messages<PlaySfx>>()
                    .drain()
                    .map(|sound| sound.place),
            );
        }
        heard
    };

    assert_eq!(
        heard(&mut app, 4),
        Vec::new(),
        "no blow, no flinch: not even at blows landed before it was seen"
    );
    app.world_mut().entity_mut(struck).insert(Hurt(1));
    let after_one = heard(&mut app, 12);
    app.world_mut().entity_mut(struck).insert(Hurt(2));
    app.world_mut().entity_mut(arrived).insert(Hurt(3));
    let after_two = heard(&mut app, 12);
    let at_struck = SfxPlace::World(Pos::new(1.5, 1.5));
    assert_eq!(after_one, vec![at_struck], "a blow, a flinch, played once");
    assert_eq!(after_two, vec![at_struck], "the next blow, the next flinch");
}

#[test]
fn a_body_talks_only_while_its_line_types_and_makes_the_gesture_it_is_said_with() {
    let clips: Vec<(&str, engine::core::time::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("cheer", CLIP.seconds())])
        .collect();
    let base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| (action.as_ref().to_owned(), json!(action.as_ref())))
            .collect();
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("chatter")
        .sfx("yay")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base, "gestures": { "happy": "cheer" } },
                "cues": {
                    "attack": [{ "at": APEX.0, "apex": true }],
                    "talk": [{ "at": 50.0, "sound": path("sfx", "chatter") }],
                    "cheer": [{ "at": 50.0, "sound": path("sfx", "yay") }],
                },
            }),
        )
        .hero("field", 2.0, 2.0);
    let content = game.load();
    let chatter = testkit::Declared::declared::<SfxDef>(&content, "chatter").expect("chatter");
    let yay = testkit::Declared::declared::<SfxDef>(&content, "yay").expect("yay");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )))
        .init_resource::<Animator>()
        .add_message::<PlaySfx>()
        .insert_resource(content.clone());
    let hero = spawn_hero(&mut app, &content, Action::Talk);
    let heard = |app: &mut App, frames: usize| {
        let mut heard = Vec::new();
        for _ in 0..frames {
            app.update();
            app.world_mut()
                .run_system_cached(render::pose)
                .expect("posed");
            heard.extend(
                app.world_mut()
                    .resource_mut::<Messages<PlaySfx>>()
                    .drain()
                    .map(|sound| sound.id),
            );
        }
        heard
    };
    let say = |app: &mut App, gesture: Option<&str>| {
        let since = engine::core::time::Seconds(app.world().resource::<Time>().elapsed_secs());
        app.world_mut()
            .spawn((
                Typewriter::default(),
                render::Said {
                    by: hero,
                    gesture: gesture.map(GestureName::new),
                    since,
                },
            ))
            .id()
    };

    assert_eq!(
        heard(&mut app, 12),
        Vec::new(),
        "talking with nothing said, it stands idle"
    );
    let line = say(&mut app, None);
    let typing = heard(&mut app, 12);
    assert!(
        !typing.is_empty() && typing.iter().all(|id| *id == chatter),
        "it talks while its line types: {typing:?}"
    );
    finish(&mut app, line);
    assert_eq!(heard(&mut app, 12), Vec::new(), "said, it rests");
    let happy = say(&mut app, Some("happy"));
    let said_happily = heard(&mut app, 4);
    assert_eq!(said_happily, vec![yay], "a happy line cheers, once");
    let talked = heard(&mut app, 12);
    assert!(
        !talked.is_empty() && talked.iter().all(|id| *id == chatter),
        "then it talks while the line types on: {talked:?}"
    );
    finish(&mut app, happy);
    let sad = say(&mut app, Some("sad"));
    let said_sadly = heard(&mut app, 12);
    assert!(
        !said_sadly.is_empty() && said_sadly.iter().all(|id| *id == chatter),
        "a line said with a gesture its body names none for is only talked: {said_sadly:?}"
    );
    finish(&mut app, sad);
}

fn finish(app: &mut App, line: Entity) {
    if let Some(mut typewriter) = app.world_mut().get_mut::<Typewriter>(line) {
        typewriter.finish();
    }
}

/// The joint of the standing body's rig named `name`, once the rig stands.
fn joint(app: &mut App, name: &str) -> Option<Entity> {
    app.world_mut()
        .query::<&Body>()
        .iter(app.world())
        .find_map(|body| body.joint(name))
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
