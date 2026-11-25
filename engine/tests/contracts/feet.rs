use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::TransformPlugin;
use engine::core::ground::{GroundSize, Heightfield, SAMPLES_PER_METER, WaterSurface};
use engine::core::math::Pos;
use engine::core::render::GroundHeights;
use engine::core::render::world::layers::{
    Feet, Gait, JointLayer, JointLayersPlugin, Layered, Rig, Stepping,
};
use engine::core::units::Meters;
use std::time::Duration;
use testkit::HEIGHTS;

/// A body walking over a slope leans as far as its skeleton lets it, front to back and side to
/// side, each its own limit: a horse leans with a hill it climbs and stays upright across one, and
/// a spider clings to the ground either way. Whichever way it leans, its legs stand plumb under its
/// hips, its feet as far from them as on flat ground.
#[test]
fn a_body_on_a_slope_leans_within_its_limits_on_legs_that_stand_plumb() {
    let rise = 0.3_f32;
    let on_flat = walk(0.0, 0.0, UPRIGHT_ACROSS, WALK, None);
    for (case, across, along, most_roll, roll, pitch) in [
        ("upright, up a hill", 0.0, rise, 0.0, 0.0, rise.atan()),
        ("upright, across a hill", rise, 0.0, 0.0, 0.0, 0.0),
        ("clinging, across a hill", rise, 0.0, 0.5, rise.atan(), 0.0),
        (
            "upright, up a hill steeper than it leans",
            0.0,
            0.8,
            0.0,
            0.0,
            0.5_f32.atan(),
        ),
    ] {
        let walked = walk(across, along, most_roll, WALK, None);
        let settled = &walked.leans[SETTLING..];
        let worst = |lean: fn(&Lean) -> f32, wanted: f32| {
            settled
                .iter()
                .map(|frame| (lean(frame) - wanted).abs())
                .fold(0.0, f32::max)
        };
        let (rolls, pitches) = (
            worst(|lean| lean.roll.abs(), roll),
            worst(|lean| lean.pitch, pitch),
        );
        assert!(
            rolls < LEANS_TRUE && pitches < LEANS_TRUE,
            "{case}: the body rolls up to {rolls} rad and pitches up to {pitches} rad from the \
             {roll} rad and {pitch} rad it should"
        );
        let apart = (walked.feet_under_hips() - on_flat.feet_under_hips()).length();
        assert!(
            apart < PLUMB,
            "{case}: its feet stand {apart} m further from under its hips than on flat ground"
        );
    }
}

/// A body running on faster than its feet can step, moving more than a stride while a foot is in
/// the air, lets its clip carry its feet rather than leave them planted behind it: they stay under
/// it, on the ground, as high above the ground beneath them as the clip lifts them, and the body
/// still leans with a hill it runs up.
#[test]
fn a_body_running_faster_than_its_feet_step_keeps_them_under_it() {
    for (case, along) in [("on flat ground", 0.0_f32), ("up a hill", 0.3)] {
        let ran = walk(0.0, along, UPRIGHT_ACROSS, RUN, None);
        let strayed = ran
            .from_hips
            .iter()
            .map(|off| off.length())
            .fold(0.0, f32::max);
        assert!(
            strayed < STRIDE,
            "running {case}, a foot strays {strayed} m from under its hip"
        );
        let floats = ran.gaps.iter().map(|gap| gap.abs()).fold(0.0, f32::max);
        assert!(
            floats < ON_THE_GROUND,
            "running {case}, a foot stands {floats} m off the ground"
        );
        let pitches = ran.leans[SETTLING..]
            .iter()
            .map(|lean| (lean.pitch - along.atan()).abs())
            .fold(0.0, f32::max);
        assert!(
            pitches < LEANS_TRUE,
            "running {case}, the body pitches {pitches} rad from the hill's slope"
        );
    }
}

/// A body that comes to rest, from a walk or a run, on flat ground or on a hill, steps its feet
/// back under it rather than stand on them spread as its last stride left them.
#[test]
fn a_body_come_to_rest_squares_its_feet_under_it() {
    for (case, along, pace) in [
        ("walking on flat ground", 0.0, WALK),
        ("walking up a hill", 0.3, WALK),
        ("running up a hill", 0.3, RUN),
    ] {
        let stood = walk(0.0, along, UPRIGHT_ACROSS, pace, Some(STOPS));
        let last = &stood.from_hips[stood.from_hips.len() - HIPS.len()..];
        let spread = last.iter().map(|off| off.length()).fold(0.0, f32::max);
        assert!(
            spread < LIFT,
            "{case}, then still, a foot stands {spread} m from under its hip"
        );
    }
}

const SIZE: GroundSize = GroundSize {
    width: 64,
    height: 64,
};
const HIP_HEIGHT: f32 = 0.9;
const LEG: [Vec3; 2] = [Vec3::new(0.0, -0.45, 0.35), Vec3::new(0.0, -0.45, -0.35)];
const HIPS: [Vec3; 4] = [
    Vec3::new(-0.3, 0.0, 0.8),
    Vec3::new(0.3, 0.0, 0.8),
    Vec3::new(-0.3, 0.0, -0.8),
    Vec3::new(0.3, 0.0, -0.8),
];
/// A walk's pace and a run's, in metres a second, either side of the stride a foot can step while
/// in the air.
const WALK: f32 = 1.5;
const RUN: f32 = 6.0;
const STRIDE: f32 = 0.5;
const STEP: f32 = 0.25;
const LIFT: f32 = 0.1;
/// The frame a body that comes to rest stops on.
const STOPS: usize = 160;
const MOST_PITCH: f32 = 0.5;
const UPRIGHT_ACROSS: f32 = 0.0;
/// How many frames the body is given to settle onto the slope.
const SETTLING: usize = 120;
/// How far the body's lean may stray from the slope it should take, in radians: its feet step,
/// and it settles anew onto each footing.
const LEANS_TRUE: f32 = 0.06;
/// How much further, on average, the feet may stand from under their hips than on flat ground.
const PLUMB: f32 = 0.08;
/// How far a foot may stand off the ground: the leg's reach rounds.
const ON_THE_GROUND: f32 = 0.03;

#[derive(Component)]
struct Body;

#[derive(Resource)]
struct Ground(Heightfield);

#[derive(Resource)]
struct Pace(f32);

#[derive(Resource)]
struct Rest(Vec<(Entity, Transform)>);

/// How the body's back leans in a frame, in radians: to its right side, and nose up.
struct Lean {
    roll: f32,
    pitch: f32,
}

struct Walked {
    leans: Vec<Lean>,
    /// How far each foot stands from under its hip on the ground, frame by frame once settled.
    from_hips: Vec<Vec2>,
    /// How far above the ground under it each foot stands, frame by frame once settled.
    gaps: Vec<f32>,
}

impl Walked {
    /// Where the feet stand from under their hips, on average.
    fn feet_under_hips(&self) -> Vec2 {
        self.from_hips.iter().sum::<Vec2>() / self.from_hips.len() as f32
    }
}

/// Walks a four-legged body forward (+Z) at `pace` over ground rising `across` a metre to its
/// right (+X) and `along` a metre ahead, its lean to its side held to `most_roll`, stopping at the
/// frame `stops` if given.
fn walk(across: f32, along: f32, most_roll: f32, pace: f32, stops: Option<usize>) -> Walked {
    let heights = Heightfield::from_fn(SIZE, HEIGHTS, |column, row| {
        let x = column as f32 / SAMPLES_PER_METER as f32 - 32.0;
        let z = row as f32 / SAMPLES_PER_METER as f32 - 32.0;
        Meters(4.0 + x * across + z * along)
    });
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, JointLayersPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )))
        .insert_resource(GroundHeights {
            heights: heights.clone(),
            water: WaterSurface::dry(SIZE, HEIGHTS),
            bounds: SIZE.bounds(),
        })
        .insert_resource(Ground(heights))
        .insert_resource(Pace(pace));
    let body = app
        .world_mut()
        .spawn((Body, Transform::from_xyz(31.5, 0.0, 26.0)))
        .id();
    let base_rest = Transform::from_xyz(0.0, HIP_HEIGHT, 0.0);
    let base = app.world_mut().spawn((base_rest, ChildOf(body))).id();
    let mut joints = vec![base];
    let mut rest = vec![base_rest];
    let mut parents = vec![None];
    let mut legs = Vec::new();
    let mut ends = Vec::new();
    for hip in HIPS {
        let mut leg = Vec::new();
        let mut parent = (base, 0_u16);
        for offset in std::iter::once(hip).chain(LEG) {
            let placed = Transform::from_translation(offset);
            let joint = app.world_mut().spawn((placed, ChildOf(parent.0))).id();
            let index = joints.len() as u16;
            joints.push(joint);
            rest.push(placed);
            parents.push(Some(parent.1));
            leg.push(index);
            parent = (joint, index);
        }
        legs.push(leg);
        ends.push((hip, joints[joints.len() - 1]));
    }
    app.insert_resource(Rest(joints.iter().copied().zip(rest.clone()).collect()));
    let rig = Rig {
        joints,
        parents,
        rest,
    };
    let feet = Feet::new(
        &rig,
        &legs,
        Gait::Diagonal,
        Stepping {
            stride: STRIDE,
            lift: LIFT,
            step: STEP,
            lead: 0.5,
            most_rise: 0.5,
            most_pitch: MOST_PITCH,
            most_roll,
            settle: 0.12,
        },
    );
    let mut layered = Layered::new(rig, vec![JointLayer::Feet(feet)]);
    layered.on = vec![true];
    app.world_mut().entity_mut(body).insert(layered);
    app.add_systems(Update, (play_clip, walk_on));
    let mut walked = Walked {
        leans: Vec::new(),
        from_hips: Vec::new(),
        gaps: Vec::new(),
    };
    for frame in 0..SETTLING + 180 {
        if stops == Some(frame) {
            app.insert_resource(Pace(0.0));
        }
        app.update();
        let placed = |entity: Entity| {
            *app.world()
                .get::<GlobalTransform>(entity)
                .expect("a placed joint")
        };
        let back = placed(base);
        let (right, ahead) = (back.right().as_vec3(), back.back().as_vec3());
        walked.leans.push(Lean {
            roll: right.y.asin(),
            pitch: ahead.y.asin(),
        });
        if frame >= SETTLING {
            let stands = placed(body).translation();
            for (hip, foot) in &ends {
                let under = stands + *hip;
                let at = placed(*foot).translation();
                walked
                    .from_hips
                    .push(Vec2::new(at.x - under.x, at.z - under.z));
                let ground = app
                    .world()
                    .resource::<Ground>()
                    .0
                    .height_at(Pos::new(at.x, at.z));
                walked.gaps.push(at.y - ground.0);
            }
        }
    }
    walked
}

/// Sets every joint as the clip keys it, before the layers run.
fn play_clip(rest: Res<Rest>, mut transforms: Query<&mut Transform>) {
    for (joint, pose) in &rest.0 {
        if let Ok(mut transform) = transforms.get_mut(*joint) {
            *transform = *pose;
        }
    }
}

/// Walks the body on at its pace, standing on the ground under it.
fn walk_on(
    time: Res<Time>,
    ground: Res<Ground>,
    pace: Res<Pace>,
    mut body: Query<&mut Transform, With<Body>>,
) {
    let Ok(mut body) = body.single_mut() else {
        return;
    };
    body.translation.z += pace.0 * time.delta_secs();
    let under = ground
        .0
        .height_at(Pos::new(body.translation.x, body.translation.z));
    body.translation.y = under.0;
}
