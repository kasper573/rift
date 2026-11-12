use std::collections::BTreeMap;
use std::iter;
use std::time::{Duration, Instant};

use bevy::animation::AnimationPlugin;
use bevy::asset::AssetPlugin;
use bevy::gltf::GltfPlugin;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::world_serialization::WorldSerializationPlugin;
use engine::core::audio::playback::{PlaySfx, SfxDef, SfxId, SfxPlace};
use engine::core::content::gltf::PLACEHOLDER_LIFT;
use engine::core::ground::{GroundSize, HeightRange, Heightfield, SAMPLES_PER_METER, TileAt};
use engine::core::math::{Pos, Rect};
use engine::core::render::camera::ground_hit;
use engine::core::render::world::body::{Body, BodyPlugin};
use engine::core::render::world::layers::JointLayersPlugin;
use engine::core::render::world::material::WorldMaterial;
use engine::core::render::{
    Animator, CameraPreferences, CameraRig, Shot, capsule_hit, facing_rotation, ground_pos,
    world_point,
};
use engine::core::text::Typewriter;
use engine::core::time::UnixMillis;
use engine::core::units::{
    Degrees, MeterPos, Meters, MetersPerSec, PlaybackRate, Radians, Seconds,
};
use engine::systems::actor::held::Held;
use engine::systems::actor::render::{self, HeldProp, RenderActor};
use engine::systems::actor::skeleton::GestureName;
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Action, Actor, Appearance, Hurt, ModelDef, Motion, Playing, Pose};
use engine::systems::area::{self, AreaDef, AreaTag};
use engine::systems::item::ItemDef;
use engine::systems::movement::RenderPosition;
use serde_json::json;
use testkit::{APEX, BODY, CLIP, Game, HEIGHTS, Map, WALK_PACE, path, png};

const SIZE: GroundSize = GroundSize {
    width: 32,
    height: 32,
};
const WINDOW: Vec2 = Vec2::new(1280.0, 720.0);

/// A hill four metres high in the middle of the ground, sloping down a metre every two.
fn hill() -> Heightfield {
    hill_among(HEIGHTS)
}

/// The hill on ground declared to span `heights`.
fn hill_among(heights: HeightRange) -> Heightfield {
    Heightfield::from_fn(SIZE, heights, |column, row| {
        let x = column as f32 / SAMPLES_PER_METER as f32;
        let y = row as f32 / SAMPLES_PER_METER as f32;
        let out = ((x - 16.0).powi(2) + (y - 16.0).powi(2)).sqrt();
        Meters((4.0 - out / 2.0).max(0.0))
    })
}

#[test]
fn the_camera_eases_toward_a_turn_and_comes_to_rest_on_it_at_any_frame_rate() {
    let framing = testkit::look().camera;
    let turned = |frames: &[f32], preferences: &CameraPreferences| {
        let mut rig = CameraRig::new(framing);
        rig.turn(Degrees(90.0));
        rig.tilt(Degrees(-10.0));
        let mut yaws = Vec::new();
        for &frame in frames {
            rig.ease(Seconds(frame), preferences);
            yaws.push(rig.yaw().0);
        }
        (rig, yaws)
    };
    let smooth = CameraPreferences::default();
    let wanted = Degrees(90.0).radians().0;

    let (_, yaws) = turned(&[1.0 / 60.0; 30], &smooth);
    let steps: Vec<f32> = iter::once(yaws[0])
        .chain(yaws.windows(2).map(|pair| pair[1] - pair[0]))
        .collect();
    assert!(
        yaws[0] > 0.0 && yaws[0] < wanted / 2.0,
        "the camera sets off toward the turn rather than jumping onto it: {yaws:?}"
    );
    assert!(
        steps
            .windows(2)
            .all(|pair| pair[1] > 0.0 && pair[1] < pair[0]),
        "it keeps turning, slower as it nears: {steps:?}"
    );

    let (slow, _) = turned(&[1.0 / 30.0; 6], &smooth);
    let (fast, _) = turned(&[1.0 / 120.0; 24], &smooth);
    assert!(
        (slow.yaw().0 - fast.yaw().0).abs() < 1e-3
            && (slow.pitch().0 - fast.pitch().0).abs() < 1e-3,
        "a fifth of a second turns it alike at 30 and 120 frames a second: {:?} {:?}",
        (slow.yaw(), slow.pitch()),
        (fast.yaw(), fast.pitch())
    );

    let (rested, _) = turned(&[1.0 / 60.0; 600], &smooth);
    assert!(
        rested.settled() && rested.yaw().0 == wanted,
        "given time, it comes to rest on the turn: {:?}",
        rested.yaw()
    );

    let unsmoothed = CameraPreferences {
        turn_ease: Some(Seconds(0.0)),
    };
    let (at_once, _) = turned(&[1.0 / 60.0], &unsmoothed);
    assert!(
        at_once.yaw().0 == wanted,
        "a player who turns smoothing off has the turn at once: {:?}",
        at_once.yaw()
    );
}

/// A shot is a place of the camera's own: it glides there and back without a jolt, and where the
/// player keeps it is the same after as before.
#[test]
fn a_camera_put_on_a_shot_glides_there_and_back_to_where_the_player_kept_it() {
    for (east, south) in [(10.0, 10.0), (1360.0, 7040.0), (52000.0, 97000.0)] {
        glides_onto_a_shot_and_back(Pos::new(east, south));
    }
}

fn glides_onto_a_shot_and_back(at: Pos<Meters>) {
    let smooth = CameraPreferences::default();
    let frame = Seconds(1.0 / 60.0);
    let mut rig = CameraRig::new(testkit::look().camera);
    rig.look_at(at, Meters(1.0));
    rig.turn(Degrees(40.0));
    for _ in 0..600 {
        rig.ease(frame, &smooth);
    }
    let kept = rig.transform();
    let shot = Shot {
        eye: Vec3::new(at.x + 2.0, 1.7, at.y - 2.0),
        target: Vec3::new(at.x, 1.4, at.y),
    };

    rig.stage(Some(shot));
    let mut left = vec![rig.eye().distance(shot.eye)];
    for _ in 0..120 {
        rig.ease(frame, &smooth);
        left.push(rig.eye().distance(shot.eye));
    }
    let steps: Vec<f32> = left.windows(2).map(|pair| pair[0] - pair[1]).collect();
    let fastest = steps
        .iter()
        .position(|step| *step == steps.iter().copied().fold(0.0, f32::max))
        .expect("a step");
    assert!(
        steps.iter().all(|step| *step >= 0.0) && fastest > 0 && fastest < steps.len() - 1,
        "at {at:?} it sets off gently, never passes the shot, and slows onto it: {steps:?}"
    );
    assert!(
        rig.settled() && rig.staged() && rig.transform() == shot.transform(),
        "at {at:?} two seconds are more than enough to be on the shot: {:?}",
        rig.transform()
    );
    assert_eq!(
        rig.kept().transform(),
        kept,
        "at {at:?} where the player keeps it stands as it was"
    );

    rig.stage(None);
    for _ in 0..120 {
        rig.ease(frame, &smooth);
    }
    assert!(
        rig.settled() && !rig.staged() && rig.transform() == kept,
        "at {at:?} off the shot it is back where the player kept it: {:?}",
        rig.transform()
    );
}

/// The camera never stands under the ground. Where the ground rises between it and what it looks
/// at, it tilts further down at once, until its line of sight passes over; over level ground it
/// stands just as it was asked; and once the ground is out of its way it settles back as asked.
#[test]
fn the_camera_tilts_over_ground_that_would_stand_in_its_sight() {
    let heights = hill();
    let preferences = &CameraPreferences::default();
    let beside = Pos::new(16.0, 6.0);
    let over = |from: Vec3, to: Vec3| (1..=200).map(move |step| from.lerp(to, step as f32 / 200.0));
    let cleared = |from: Vec3, to: Vec3| {
        over(from, to).all(|point| point.y > heights.height_at(ground_pos(point)).0)
    };
    let level = |from: Vec3, to: Vec3| {
        over(from, to).all(|point| heights.height_at(ground_pos(point)).0 == 0.0)
    };
    let asked_of = |at: Pos<Meters>, turned: f32, zoomed_out: bool| {
        let mut rig = CameraRig::new(testkit::look().camera);
        rig.look_at(at, heights.height_at(at) + Meters(1.0));
        rig.turn(Degrees(turned));
        rig.tilt(Degrees(-90.0));
        if zoomed_out {
            rig.zoom_out();
        }
        rig.ease(Seconds(60.0), preferences);
        rig
    };
    let open = Pos::new(4.0, 4.0);
    let away = 225.0;
    let (mut tilted, mut kept) = (0, 0);
    for turned in (0..360).step_by(15).map(|turned| turned as f32) {
        for zoomed_out in [false, true] {
            let mut rig = asked_of(beside, turned, zoomed_out);
            let asked = rig.eye();
            rig.stand_over(&heights, Seconds(1.0 / 60.0), preferences);
            let eye = rig.eye();
            assert!(
                cleared(rig.target(), eye),
                "turned {turned}°, the line of sight from {eye} passes over the ground"
            );
            if level(rig.target(), asked) {
                assert_eq!(
                    eye, asked,
                    "turned {turned}°, level ground leaves the camera be"
                );
                kept += 1;
            } else if !cleared(rig.target(), asked) {
                assert!(
                    eye.y > asked.y,
                    "turned {turned}°, the camera tilts up over the hill: {eye} from {asked}"
                );
                tilted += 1;
            }

            rig.look_at(open, heights.height_at(open) + Meters(1.0));
            rig.turn(Degrees(away - turned));
            rig.ease(Seconds(60.0), preferences);
            rig.stand_over(&heights, Seconds(60.0), preferences);
            let asked = asked_of(open, away, zoomed_out).eye();
            assert!(
                rig.eye().abs_diff_eq(asked, 1e-3),
                "away from the hill, the camera settles back as asked: {} for {asked}",
                rig.eye()
            );
        }
    }
    assert!(
        tilted > 0 && kept > 0,
        "the hill stands in the camera's sight from some sides only: {tilted} tilted, {kept} kept"
    );
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
        let mut rig = CameraRig::new(testkit::look().camera);
        rig.look_at(Pos::new(14.0, 14.0), Meters(1.0));
        rig.turn(Degrees(turned));
        rig.tilt(Degrees(tilted));
        rig.ease(Seconds(60.0), &CameraPreferences::default());
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

    // An area may declare room for peaks far above the ground a view looks at, and a view may
    // stand far off, as one drawn back over a whole area does.
    let lowland = hill_among(HeightRange {
        lowest: Meters(-20.0),
        highest: Meters(500.0),
    });
    for (at, name) in spots {
        let ground = world_point(at, lowland.height_at(at));
        let eye = ground + Vec3::new(60.0, 2400.0, 40.0);
        let ray = Ray3d::new(eye, Dir3::new(ground - eye).expect("a heading"));
        let landed = ground_hit(&lowland, bounds, ray)
            .unwrap_or_else(|| panic!("the ray from far above {name} lands"));
        assert!(
            landed.distance(at) < Meters(0.05),
            "pointing at {name} from far above landed {landed:?}, not {at:?}"
        );
    }
}

#[test]
fn pointing_at_the_ground_meets_it_where_a_fine_march_does() {
    // Three tiles by two of rolling ground, held as play holds it: one tile read, the others
    // answered by the overview until they are.
    let size = GroundSize {
        width: 600,
        height: 300,
    };
    let rolling = Heightfield::from_fn(size, HEIGHTS, |column, row| {
        let x = column as f32 / SAMPLES_PER_METER as f32;
        let y = row as f32 / SAMPLES_PER_METER as f32;
        Meters(4.0 * (x / 23.0).sin() * (y / 17.0).cos() + 2.5 * ((x + y) / 9.0).sin() + 3.0)
    });
    let game = size.each_tile().fold(
        Game::new()
            .area("rolling", &Map::open(600, 300), json!([]))
            .part(
                "area",
                "rolling",
                "height_overview.png",
                rolling.encode_overview().expect("heights that aren't flat"),
            ),
        |game, at| {
            let tile = rolling.encode_tile(at).expect("heights that aren't flat");
            game.part(
                "area",
                "rolling",
                &format!("height/{}.png", at.key()),
                tile.to_vec(),
            )
        },
    );
    let content = game.hero("rolling", 1.0, 1.0).load_to_play();
    let rolling = content
        .by_path::<area::AreaDef>(&path("area", "rolling"))
        .expect("the area loads");
    let mut heights = area::load(&content, rolling).heights.clone();
    heights
        .load(TileAt { column: 1, row: 0 })
        .expect("the tile reads");
    let bounds = size.bounds().inflate(2.0, 2.0);
    let mound = Pos::new(300.0, 100.0);
    let pit = Pos::new(380.0, 60.0);

    let mut rays = Vec::new();
    for x in [-10.0, 120.0, 255.5, 400.0, 511.5, 610.0] {
        for z in [-10.0, 150.0, 255.5, 310.0] {
            let eye = Vec3::new(x, 40.0, z);
            for yaw in (0..8).map(|step| step as f32 * 45.0 + 10.0) {
                for pitch in [-15.0f32, -35.0, -70.0] {
                    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
                    let heading = Vec3::new(
                        yaw.cos() * pitch.cos(),
                        pitch.sin(),
                        yaw.sin() * pitch.cos(),
                    );
                    rays.push(Ray3d::new(eye, Dir3::new(heading).expect("a heading")));
                }
            }
        }
    }
    for (at, height) in [(mound, 13.0), (pit, -5.0)] {
        for side in 0..6 {
            let side = side as f32 * 1.05;
            let eye = Vec3::new(at.x + 30.0 * side.cos(), 30.0, at.y + 30.0 * side.sin());
            let aim = Vec3::new(at.x, height, at.y) - eye;
            rays.push(Ray3d::new(eye, Dir3::new(aim).expect("a heading")));
        }
    }

    let agree = |heights: &Heightfield, held: &str| {
        for ray in &rays {
            let found = ground_hit(heights, bounds, *ray);
            let marched = marched(heights, bounds, *ray);
            match (found, marched) {
                (Some(found), Some(marched)) => assert!(
                    found.distance(marched) < Meters(0.01),
                    "{held}: the ray from {:?} heading {:?} meets the ground at {found:?}, a \
                     fine march at {marched:?}",
                    ray.origin,
                    ray.direction
                ),
                (found, marched) => assert_eq!(
                    found, marched,
                    "{held}: the ray from {:?} heading {:?}",
                    ray.origin, ray.direction
                ),
            }
        }
    };
    agree(&heights, "a tile read, the rest from the overview");
    heights
        .load(TileAt { column: 2, row: 0 })
        .expect("the tile reads");
    agree(&heights, "another tile read since");

    let samples = |at: Pos<Meters>| {
        let column = ((at.x + 0.5) * SAMPLES_PER_METER as f32) as u32;
        let row = ((at.y + 0.5) * SAMPLES_PER_METER as f32) as u32;
        (column - 12..column + 12)
            .flat_map(move |column| (row - 12..row + 12).map(move |row| (column, row)))
    };
    for (column, row) in samples(mound) {
        heights.set_sample(column, row, Meters(13.0));
    }
    for (column, row) in samples(pit) {
        heights.set_sample(column, row, Meters(-5.0));
    }
    agree(&heights, "a mound raised and a pit dug");
}

/// Where a ray first comes down to the ground, marched a couple of centimetres at a time and the
/// crossing then halved down to nothing; none once it has passed under the lowest ground there is.
fn marched(heights: &Heightfield, bounds: Rect<Meters>, ray: Ray3d) -> Option<Pos<Meters>> {
    const STEP: f32 = 0.02;
    let under = |t: f32| {
        let point = ray.get_point(t);
        let at = Pos::new(point.x, point.z);
        bounds.contains(at) && point.y <= heights.height_at(at).0
    };
    let mut t = 0.0;
    while t < 400.0 {
        if ray.get_point(t + STEP).y < heights.range().lowest.0 {
            return None;
        }
        if under(t + STEP) {
            let (mut above, mut below) = (t, t + STEP);
            for _ in 0..24 {
                let middle = (above + below) / 2.0;
                if under(middle) {
                    below = middle;
                } else {
                    above = middle;
                }
            }
            let point = ray.get_point(below);
            return Some(Pos::new(point.x, point.z));
        }
        t += STEP;
    }
    None
}

#[test]
fn pointing_at_a_body_where_it_shows_picks_it() {
    const RADIUS: f32 = 0.4;
    const HEIGHT: f32 = 2.3;
    let heights = hill();
    let at = Pos::new(12.0, 19.0);
    let foot = world_point(at, heights.height_at(at));
    for zoomed in 0..3 {
        let mut rig = CameraRig::new(testkit::look().camera);
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
fn a_body_wears_the_paint_its_material_bakes_from_the_games_images() {
    const CLOTH: [u8; 3] = [200, 30, 40];
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .hero("field", 2.0, 2.0)
        .edit("skeleton", BODY, |body| {
            body["tints"] = json!([{ "name": "cloth", "colour": CLOTH }])
        })
        .row(
            "material",
            "cloth",
            json!({
                "skeleton": path("skeleton", BODY),
                "base": "materials/base.png",
                "masks": [{ "image": "materials/mask.png", "inverted": true, "r": "cloth" }],
            }),
        )
        .asset("materials/base.png", png(4, 4))
        .asset("materials/mask.png", png(4, 4))
        .edit("model_part", "hero", |part| {
            part["material"] = json!(path("material", "cloth"))
        });
    let folder = tempfile::tempdir().expect("a scratch folder");
    let (mut app, content) = viewer(game, folder.path());
    app.insert_resource(testkit::look());
    spawn_hero(&mut app, &content, Action::Idle);
    app.world_mut()
        .run_system_cached(render::dress)
        .expect("dressed");

    let worn = |app: &App| {
        app.world()
            .resource::<Assets<Image>>()
            .iter()
            .any(|(_, image)| {
                image.data.as_ref().is_some_and(|pixels| {
                    pixels.len() == 4 * 4 * 4
                        && pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|pixel| pixel[..3] == CLOTH)
                })
            })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worn(&app) && Instant::now() < deadline {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(worn(&app), "the body's texture is painted in its cloth");
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
        .edit(
            "skeleton",
            BODY,
            |skeleton| {
                skeleton["hands"][0]["clasp"] =
                    json!({ "clip": testkit::clip("clasp"), "fingers": ["finger.r"] })
            },
        )
        .asset(
            &testkit::clip("clasp"),
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
        bevy::mesh::MeshPlugin,
        BodyPlugin,
        JointLayersPlugin,
    ))
    .init_asset::<StandardMaterial>()
    .init_asset::<WorldMaterial>()
    .init_asset::<Image>()
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
        .sfx("thud")
        .skeleton_of(
            BODY,
            &testkit::rig_clips(),
            json!({ "cues": [
                { "clip": testkit::clip("walk"), "cues": [{ "at": 100.0, "step": true }] },
                {
                    "clip": testkit::clip("attack"),
                    "cues": [{ "at": APEX.0, "apex": true, "sound": path("sfx", "whoosh") }],
                },
            ] }),
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
    let worker = Pos::new(3.5, 1.5);
    let thud = content
        .by_path::<SfxDef>(&path("sfx", "thud"))
        .expect("the thud sound");
    for (at, action, speed) in [
        (walker, Action::Walk, WALK_PACE),
        (hurrier, Action::Walk, WALK_PACE * 2.0),
        (striker, Action::Attack, MetersPerSec::default()),
        (worker, Action::Attack, MetersPerSec::default()),
    ] {
        let stroke = (at == worker).then_some(Playing {
            motion: Motion::Action(Action::Attack),
            pick: 0,
            started: UnixMillis(0),
            apex: UnixMillis(0),
            ends: UnixMillis(0),
            sound: Some(thud),
        });
        let mut body = app.world_mut().spawn((
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
        if let Some(stroke) = stroke {
            body.insert(stroke);
        }
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
                    SfxPlace::Afar { .. } | SfxPlace::Interface => {
                        panic!("a body sounds where it stands")
                    }
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
    assert_eq!(
        of(worker),
        vec![thud],
        "a stroke that lands with a sound of its own sounds that in place of its clip's"
    );
}

#[test]
fn a_looping_cells_clips_take_turns_each_as_the_one_before_plays_through() {
    let clips: Vec<(&str, engine::core::units::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("hum", CLIP.seconds()), ("tap", CLIP.seconds())])
        .collect();
    let mut base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| {
                (
                    action.as_ref().to_owned(),
                    json!(testkit::clip(action.as_ref())),
                )
            })
            .collect();
    base.insert(
        "idle".to_owned(),
        json!({ "clips": [testkit::clip("hum"), testkit::clip("tap")], "order": "in_turn" }),
    );
    let cue = |sound: &str| json!({ "clip": testkit::clip(sound), "cues": [{ "at": 100.0, "sound": path("sfx", sound) }] });
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("hum")
        .sfx("tap")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base },
                "cues": [attacked(), cue("hum"), cue("tap")],
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
    let clips: Vec<(&str, engine::core::units::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("flinch", CLIP.seconds())])
        .collect();
    let mut base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .filter(|action| **action != Action::Hurt)
            .map(|action| {
                (
                    action.as_ref().to_owned(),
                    json!(testkit::clip(action.as_ref())),
                )
            })
            .collect();
    base.insert("hurt".to_owned(), json!(testkit::clip("flinch")));
    let stoic_base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .filter(|action| **action != Action::Hurt)
            .map(|action| {
                let clip = testkit::clip_on("stoic", action.as_ref());
                (action.as_ref().to_owned(), json!(clip))
            })
            .collect();
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("ow")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base },
                "cues": [
                    attacked(),
                    {
                        "clip": testkit::clip("flinch"),
                        "cues": [{ "at": 50.0, "sound": path("sfx", "ow") }],
                    },
                ],
            }),
        )
        .skeleton_of(
            "stoic",
            &testkit::rig_clips(),
            json!({ "base": { "clips": stoic_base } }),
        )
        .model_on("stoic", "stoic", 1.8, json!([]))
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
    let stoic = testkit::Declared::declared::<ModelDef>(&content, "stoic").expect("a stoic");
    let field = testkit::Declared::declared::<AreaDef>(&content, "field").expect("a field");
    let mut body = |model, at: Pos<Meters>, blows: u32| {
        app.world_mut()
            .spawn((
                Actor {
                    facing: Radians(0.0),
                    action: Action::Idle,
                    pose: Pose::Idle,
                    attack_rate: PlaybackRate(1.0),
                    speed: MetersPerSec::default(),
                },
                Appearance::of(model),
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
    let struck = body(hero, Pos::new(1.5, 1.5), 0);
    let arrived = body(hero, Pos::new(3.5, 3.5), 3);
    let unflinching = body(stoic, Pos::new(2.5, 2.5), 0);
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
    app.world_mut().entity_mut(unflinching).insert(Hurt(1));
    let after_one = heard(&mut app, 12);
    app.world_mut().entity_mut(struck).insert(Hurt(2));
    app.world_mut().entity_mut(arrived).insert(Hurt(3));
    let after_two = heard(&mut app, 12);
    let at_struck = SfxPlace::World(Pos::new(1.5, 1.5));
    assert_eq!(
        after_one,
        vec![at_struck],
        "a blow, a flinch, played once, by a body that has one"
    );
    assert_eq!(after_two, vec![at_struck], "the next blow, the next flinch");
}

#[test]
fn a_body_talks_only_while_its_line_types_and_makes_the_gesture_it_is_said_with() {
    let clips: Vec<(&str, engine::core::units::Seconds)> = testkit::rig_clips()
        .into_iter()
        .chain([("cheer", CLIP.seconds())])
        .collect();
    let base: serde_json::Map<String, serde_json::Value> =
        <Action as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| {
                (
                    action.as_ref().to_owned(),
                    json!(testkit::clip(action.as_ref())),
                )
            })
            .collect();
    let game = Game::new()
        .area("field", &Map::open(5, 5), json!([]))
        .sfx("chatter")
        .sfx("yay")
        .skeleton_of(
            BODY,
            &clips,
            json!({
                "base": { "clips": base, "gestures": { "happy": testkit::clip("cheer") } },
                "cues": [
                    attacked(),
                    {
                        "clip": testkit::clip("talk"),
                        "cues": [{ "at": 50.0, "sound": path("sfx", "chatter") }],
                    },
                    {
                        "clip": testkit::clip("cheer"),
                        "cues": [{ "at": 50.0, "sound": path("sfx", "yay") }],
                    },
                ],
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
        let since = engine::core::units::Seconds(app.world().resource::<Time>().elapsed_secs());
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

/// The cues of the plain skeleton's attack: its apex.
fn attacked() -> serde_json::Value {
    json!({ "clip": testkit::clip("attack"), "cues": [{ "at": APEX.0, "apex": true }] })
}
