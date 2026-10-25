//! Feet planted where they touched the ground. Each leg's foot stays put while the body moves on,
//! and steps in an arc once the spot its clip would put it at has drifted a stride away: the legs of
//! one group of the gait together, never while another group is in the air. The body rides its
//! planted feet, raised, lowered and tilted with the ground beneath them.

use bevy_skeleton_modifiers::core::{
    BonePose, ChainIndices, IkSolver, IterateSettings, IterateState, SkeletonView, process_chain_ik,
};
use glam::{Quat, Vec2, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Frame, Rig};

/// How long a foot is in the air.
const STEP: f32 = 0.2;
/// How far past the spot the clip would put it a stepping foot lands, in strides, so it does not
/// step again at once.
const LEAD: f32 = 0.5;
/// How far the planted feet may raise or lower the body, in metres.
const MOST_RISE: f32 = 0.5;
/// How steeply the planted feet may tilt the body.
const MOST_TILT: f32 = 0.5;
/// How quickly the body settles onto its planted feet: the time it takes to go most of the way.
const SETTLE: f32 = 0.12;

/// Which feet step together.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    strum::VariantArray,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Gait {
    /// One side's legs together, then the other's: a walk.
    Lateral,
    /// Opposite corners together: a trot.
    Diagonal,
    /// Alternate legs of each side, three of six at a time.
    Tripod,
    /// Alternate legs of each side, four of eight at a time.
    Tetrapod,
    /// One leg after the next, down the body.
    Wave,
}

pub struct Feet {
    legs: Vec<Leg>,
    stride: f32,
    lift: f32,
    /// How far the body is raised and how its ground slopes under it, along its right and front.
    body: Vec3,
}

struct Leg {
    chain: ChainIndices,
    group: usize,
    /// How high above the ground the foot rests, in the rig's space.
    rest_height: f32,
    planted: Option<Vec3>,
    step: Option<Step>,
    ik: IterateState,
}

struct Step {
    from: Vec3,
    to: Vec3,
    t: f32,
}

impl Feet {
    /// The feet of `legs`, each a chain of joints from the hip to the foot down the rig, stepping in
    /// `gait` once a foot is `stride` metres from its spot, lifting it `lift` metres. A leg that is
    /// not a chain down the rig is left out.
    pub fn new(rig: &Rig, legs: &[Vec<u16>], gait: Gait, stride: f32, lift: f32) -> Feet {
        let rest = rig.rest_poses();
        let mut posed = rest.clone();
        let view = SkeletonView {
            parents: &rig.parents,
            rest: &rest,
            local: &mut posed,
            root_global: glam::Affine3A::IDENTITY,
        };
        let found: Vec<(ChainIndices, Vec3)> = legs
            .iter()
            .filter_map(|joints| {
                let (&hip, &foot) = (joints.first()?, joints.last()?);
                let chain = view.chain(hip, foot).ok()?;
                Some((chain, Vec3::from(view.skeleton_pose(foot).translation)))
            })
            .collect();
        let groups = groups(gait, &found.iter().map(|(_, at)| *at).collect::<Vec<_>>());
        Feet {
            legs: found
                .into_iter()
                .zip(groups)
                .map(|((chain, at), group)| Leg {
                    chain,
                    group,
                    rest_height: at.y,
                    planted: None,
                    step: None,
                    ik: IterateState::default(),
                })
                .collect(),
            stride,
            lift,
            body: Vec3::ZERO,
        }
    }

    pub(super) fn reset(&mut self) {
        for leg in &mut self.legs {
            leg.planted = None;
            leg.step = None;
            leg.ik.reset();
        }
        self.body = Vec3::ZERO;
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        let root = frame.view.root_global;
        let scale = root.matrix3.y_axis.length();
        self.settle(frame);
        let to_rig = root.inverse();
        let aloft: Vec<usize> = self
            .legs
            .iter()
            .filter(|leg| leg.step.is_some())
            .map(|leg| leg.group)
            .collect();
        let mut lifting: Option<usize> = aloft.first().copied();
        for leg in &mut self.legs {
            let foot = *leg.chain.as_slice().last().expect("a leg has a foot");
            let clip =
                root.transform_point3(Vec3::from(frame.view.skeleton_pose(foot).translation));
            let spot = grounded(frame, clip, leg.rest_height * scale);
            let planted = *leg.planted.get_or_insert(spot);
            let target = match &mut leg.step {
                Some(step) => {
                    step.t += frame.dt / STEP;
                    if step.t >= 1.0 {
                        let landed = step.to;
                        leg.planted = Some(landed);
                        leg.step = None;
                        landed
                    } else {
                        step.from.lerp(step.to, step.t)
                            + Vec3::Y * self.lift * scale * (step.t * std::f32::consts::PI).sin()
                    }
                }
                None => {
                    let drift = Vec2::new(spot.x - planted.x, spot.z - planted.z);
                    let free = lifting.is_none_or(|group| group == leg.group);
                    if drift.length() > self.stride * scale && free {
                        lifting = Some(leg.group);
                        let ahead = drift.normalize_or_zero() * self.stride * scale * LEAD;
                        let to = grounded(
                            frame,
                            Vec3::new(spot.x + ahead.x, spot.y, spot.z + ahead.y),
                            leg.rest_height * scale,
                        );
                        leg.step = Some(Step {
                            from: planted,
                            to,
                            t: 0.0,
                        });
                    }
                    planted
                }
            };
            process_chain_ik(
                &mut frame.view,
                &leg.chain,
                IkSolver::Fabrik,
                &IterateSettings {
                    max_iterations: 8,
                    deterministic: true,
                    ..IterateSettings::default()
                },
                &[],
                to_rig.transform_point3(target),
                &mut leg.ik,
            );
        }
    }

    /// Raises, lowers and tilts the body toward the ground under its planted feet: a plane fitted
    /// through how far each stands above or below its rest, along the body's right and front.
    fn settle(&mut self, frame: &mut Frame) {
        let root = frame.view.root_global;
        let to_rig = root.inverse();
        let scale = root.matrix3.y_axis.length().max(f32::EPSILON);
        let planted: Vec<(Vec3, f32)> = self
            .legs
            .iter()
            .filter_map(|leg| {
                let at = to_rig.transform_point3(leg.planted?);
                Some((at, at.y - leg.rest_height))
            })
            .collect();
        let wanted = match planted.len() {
            0 => Vec3::ZERO,
            1 | 2 => Vec3::new(
                planted.iter().map(|(_, rise)| rise).sum::<f32>() / planted.len() as f32,
                0.0,
                0.0,
            ),
            _ => plane(&planted),
        };
        let wanted = Vec3::new(
            wanted.x.clamp(-MOST_RISE / scale, MOST_RISE / scale),
            wanted.y.clamp(-MOST_TILT, MOST_TILT),
            wanted.z.clamp(-MOST_TILT, MOST_TILT),
        );
        self.body += (wanted - self.body) * (1.0 - (-frame.dt / SETTLE).exp());
        let Some(base) =
            (0..frame.view.parents.len()).find(|&joint| frame.view.parents[joint].is_none())
        else {
            return;
        };
        let BonePose {
            translation,
            rotation,
            scale: own,
        } = frame.view.local[base];
        let tilt =
            Quat::from_rotation_z(self.body.y.atan()) * Quat::from_rotation_x(-self.body.z.atan());
        frame.view.local[base] = BonePose {
            translation: translation + Vec3::Y * self.body.x,
            rotation: tilt * rotation,
            scale: own,
        };
    }
}

/// `at` brought down onto the ground, `height` above it.
fn grounded(frame: &Frame, at: Vec3, height: f32) -> Vec3 {
    Vec3::new(at.x, (frame.ground)(at) + height, at.z)
}

/// The rise `a` and slopes `b` along x and `c` along z of the plane `rise = a + b x + c z` closest
/// to every planted foot, as `(a, b, c)`.
fn plane(feet: &[(Vec3, f32)]) -> Vec3 {
    let n = feet.len() as f32;
    let mean = feet.iter().fold(Vec3::ZERO, |sum, (at, rise)| {
        sum + Vec3::new(at.x, *rise, at.z)
    }) / n;
    let (mut xx, mut xz, mut zz, mut xr, mut zr) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (at, rise) in feet {
        let (x, z, r) = (at.x - mean.x, at.z - mean.z, rise - mean.y);
        xx += x * x;
        xz += x * z;
        zz += z * z;
        xr += x * r;
        zr += z * r;
    }
    let det = xx * zz - xz * xz;
    if det.abs() < 1e-6 {
        return Vec3::new(mean.y, 0.0, 0.0);
    }
    let b = (xr * zz - zr * xz) / det;
    let c = (zr * xx - xr * xz) / det;
    Vec3::new(mean.y - b * mean.x - c * mean.z, b, c)
}

/// The step group of each foot resting at `feet`, by its side (left of the body's middle or right)
/// and its place from the front (+Z) back.
fn groups(gait: Gait, feet: &[Vec3]) -> Vec<usize> {
    let side = |at: &Vec3| usize::from(at.x >= 0.0);
    let rank = |index: usize| {
        let at = feet[index];
        feet.iter()
            .filter(|other| side(other) == side(&at) && other.z > at.z)
            .count()
    };
    match gait {
        Gait::Lateral => feet.iter().map(side).collect(),
        Gait::Diagonal | Gait::Tripod | Gait::Tetrapod => (0..feet.len())
            .map(|index| (side(&feet[index]) + rank(index)) % 2)
            .collect(),
        Gait::Wave => (0..feet.len())
            .map(|index| rank(index) * 2 + side(&feet[index]))
            .collect(),
    }
}
