use bevy::camera::CameraProjection;
use bevy::camera::primitives::Frustum;
use bevy::prelude::*;
use bevy::shape::ViewFrustum;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::TransformPlugin;
use engine::core::render::WorldCamera;
use engine::core::render::world::layers::{Chain, JointLayer, JointLayersPlugin, Layered, Rig};
use std::time::Duration;

/// A sprung tail behind a body running on steadily, straight or round a curve, settles into a
/// lean it holds: seen from the body it stays still frame after frame, at any frame rate, with
/// frames that come a little long or short at random, whether the camera is near or far enough
/// that the body's layers run every other frame, and whether or not the body's clip keys it. The
/// runs pair every frame rate with each way of the other three, and those three with each other.
#[test]
fn a_sprung_tail_settles_behind_a_body_running_on_steadily() {
    struct Run {
        rate: &'static str,
        frames: &'static [f32],
        running: Running,
        back: f32,
        keyed: bool,
    }
    const STRAIGHT: Running = Running {
        speed: 7.0,
        turn: 0.0,
    };
    const CURVE: Running = Running {
        speed: 7.0,
        turn: 0.6,
    };
    const ABOUT_60: &[f32] = &[0.0155, 0.0178, 0.0162, 0.0171];
    const ABOUT_40: &[f32] = &[0.024, 0.026, 0.0245, 0.0255];
    let run = |rate, frames, running, back, keyed| Run {
        rate,
        frames,
        running,
        back,
        keyed,
    };
    let runs = [
        run("60 fps", &[1.0 / 60.0], STRAIGHT, 6.0, true),
        run("60 fps", &[1.0 / 60.0], CURVE, 15.0, false),
        run("about 60 fps", ABOUT_60, STRAIGHT, 15.0, false),
        run("about 60 fps", ABOUT_60, CURVE, 6.0, true),
        run("about 40 fps", ABOUT_40, CURVE, 6.0, false),
        run("about 40 fps", ABOUT_40, STRAIGHT, 15.0, true),
        run("30 fps", &[1.0 / 30.0], CURVE, 15.0, true),
        run("30 fps", &[1.0 / 30.0], STRAIGHT, 6.0, false),
    ];
    for Run {
        rate,
        frames,
        running,
        back,
        keyed,
    } in runs
    {
        let Running { speed, turn } = running;
        let swings = swings(&tail(frames, back, keyed, running, 1.0));
        let worst = swings[SETTLING..].iter().copied().fold(0.0, f32::max);
        assert!(
            worst < STILL,
            "at {rate}, running at {speed} m/s turning {turn}, the camera {back} m back and the \
             tail {}keyed, the settled tail still swings {worst} rad in a frame",
            if keyed { "" } else { "un" },
        );
    }
}

/// A rig is a rig whatever units it was made in: a tail on a rig whose joints carry a scale (as a
/// glTF exported in centimetres does) springs exactly as the same tail made at scale one.
#[test]
fn a_sprung_tail_swings_alike_whatever_scale_its_rig_was_made_at() {
    for (speed, turn) in [(7.0, 0.0), (7.0, 0.6), (0.0, 0.0)] {
        let made_at_one = tail(&[1.0 / 60.0], 6.0, true, Running { speed, turn }, 1.0);
        let made_at_forty = tail(&[1.0 / 60.0], 6.0, true, Running { speed, turn }, 40.0);
        let worst = made_at_one
            .iter()
            .zip(&made_at_forty)
            .flat_map(|(one, forty)| one.iter().zip(forty).map(|(a, b)| a.distance(*b)))
            .fold(0.0, f32::max);
        assert!(
            worst < 0.001,
            "running at {speed} m/s turning {turn}, the tail of the rig made at forty times its \
             scale stands {worst} m from the same tail made at one"
        );
    }
}

/// How many frames a tail is given to settle into its lean.
const SETTLING: usize = 240;
/// How far a settled tail may turn in a frame, seen from the body: the spring's own rounding.
const STILL: f32 = 0.002;
const TAIL: [Vec3; 4] = [
    Vec3::new(0.0, 0.0, -0.6),
    Vec3::new(0.0, -0.05, -0.15),
    Vec3::new(0.0, -0.05, -0.15),
    Vec3::new(0.0, -0.05, -0.15),
];

#[derive(Component)]
struct Body;

#[derive(Resource)]
struct Back(f32);

#[derive(Resource)]
struct Running {
    speed: f32,
    turn: f32,
}

#[derive(Resource)]
struct Rest(Vec<(Entity, Transform)>);

/// How far the tail turns from one frame to the next, seen from the body.
fn swings(tail: &[Vec<Vec3>]) -> Vec<f32> {
    tail.windows(2)
        .map(|pair| pair[0][0].angle_between(pair[1][0]))
        .collect()
}

/// Where the tail stands, frame by frame as the frames come in the lengths `frames` over and over:
/// its direction from its first joint to its last as the body sees it, then every joint in the
/// world. Its rig is made at the scale `made_at`, its hip carrying it.
fn tail(frames: &[f32], back: f32, keyed: bool, running: Running, made_at: f32) -> Vec<Vec<Vec3>> {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, JointLayersPlugin));
    let body = app
        .world_mut()
        .spawn((Body, Transform::from_xyz(0.0, 3.0, 0.0)))
        .id();
    let hip_rest = Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::splat(made_at));
    let hip = app.world_mut().spawn((hip_rest, ChildOf(body))).id();
    let mut joints = vec![hip];
    let mut rest = vec![hip_rest];
    let mut parents = vec![None];
    for (index, offset) in TAIL.iter().enumerate() {
        let placed = Transform::from_translation(*offset / made_at);
        let joint = app
            .world_mut()
            .spawn((placed, ChildOf(*joints.last().expect("a joint"))))
            .id();
        joints.push(joint);
        rest.push(placed);
        parents.push(Some(index as u16));
    }
    let rig = Rig {
        joints: joints.clone(),
        parents,
        rest: rest.clone(),
    };
    let chain = Chain::new(&rig, 1, false, 2.0, 0.5, 1.0).expect("a chain");
    let mut layered = Layered::new(rig, vec![JointLayer::Chain(chain)]);
    layered.on = vec![true];
    app.world_mut().entity_mut(body).insert(layered);
    app.world_mut()
        .spawn((WorldCamera, GlobalTransform::default(), Frustum::default()));
    app.insert_resource(Rest(joints.iter().copied().zip(rest).collect()));
    if keyed {
        app.add_systems(Update, play_clip);
    }
    app.insert_resource(Back(back));
    app.insert_resource(running);
    app.add_systems(Update, (run_on, follow).chain());
    frames
        .iter()
        .cycle()
        .take(SETTLING + 240)
        .map(|frame| {
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                *frame,
            )));
            app.update();
            let placed = |entity: Entity| {
                *app.world()
                    .get::<GlobalTransform>(entity)
                    .expect("a placed joint")
            };
            let along = placed(joints[TAIL.len()]).translation() - placed(joints[1]).translation();
            let seen = (placed(body).rotation().inverse() * along).normalize();
            std::iter::once(seen)
                .chain(joints[1..].iter().map(|&joint| placed(joint).translation()))
                .collect()
        })
        .collect()
}

/// Sets every joint as the clip keys it, before the layers run.
fn play_clip(rest: Res<Rest>, mut transforms: Query<&mut Transform>) {
    for (joint, pose) in &rest.0 {
        if let Ok(mut transform) = transforms.get_mut(*joint) {
            *transform = *pose;
        }
    }
}

fn run_on(time: Res<Time>, running: Res<Running>, mut body: Query<&mut Transform, With<Body>>) {
    let Ok(mut body) = body.single_mut() else {
        return;
    };
    let turned = body.rotation * Quat::from_rotation_y(running.turn * time.delta_secs());
    body.rotation = turned;
    body.translation += turned * Vec3::Z * running.speed * time.delta_secs();
}

/// Keeps the camera `Back` behind and over the body, as a player's stands.
fn follow(
    back: Res<Back>,
    body: Query<&Transform, With<Body>>,
    mut camera: Query<(&mut GlobalTransform, &mut Frustum), With<WorldCamera>>,
) {
    let (Ok(body), Ok((mut placed, mut frustum))) = (body.single(), camera.single_mut()) else {
        return;
    };
    let eye = body.translation + Vec3::new(0.0, back.0 * 1.3, back.0);
    let view = Transform::from_translation(eye).looking_at(body.translation, Vec3::Y);
    *placed = GlobalTransform::from(view);
    let clip_from_world =
        PerspectiveProjection::default().get_clip_from_view() * view.to_matrix().inverse();
    *frustum = Frustum(ViewFrustum::from_clip_from_world(&clip_from_world));
}
