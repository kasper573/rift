//! A chain of joints behind the body: sprung, each joint's tail carried on by its momentum and
//! pulled back toward its pose, as VRM spring bones are; or following, each joint trailing the one
//! before it along the path the first went.

use bevy_skeleton_modifiers::core::{
    BonePose, ChainIndices, SkeletonView, SpringBoneSettings, SpringBoneState, process_spring_bones,
};
use glam::{Affine3A, Quat, Vec3};

use super::{Frame, Rig};

/// How far the first joint moves before the path it went takes another point.
const TRAIL_STEP: f32 = 0.02;

/// The step a sprung chain's stiffness, drag and gravity are given for. A frame is gone through
/// in a whole number of even steps near it, and the chain keeps the number it took while each
/// step stays within half of this of it: frames come a hair longer or shorter than it at random,
/// and a number that flipped with them would swing the chain a little differently frame by frame.
const SPRING_STEP: f32 = 1.0 / 60.0;

pub struct Chain {
    chain: ChainIndices,
    follow: bool,
    /// What the spring loses of its swing, and how hard it is pulled back toward its pose and
    /// down, over a step of `SPRING_STEP`.
    drag: f32,
    stiffness: f32,
    gravity: f32,
    spring: Box<SpringBoneSettings>,
    state: Box<SpringBoneState>,
    /// Where the first joint has been in the world, the latest first.
    trail: Vec<Vec3>,
    stood: Option<Stood>,
}

/// Where the body stood when the spring last moved on, and how it was stepped there.
#[derive(Clone, Copy)]
struct Stood {
    at: Affine3A,
    steps: u32,
    step: f32,
}

impl Chain {
    /// The joints from `from` down the rig, sprung with `stiffness`, `drag` and `gravity`, or
    /// trailing the first along its path when `follow`.
    pub fn new(
        rig: &Rig,
        from: u16,
        follow: bool,
        stiffness: f32,
        drag: f32,
        gravity: f32,
    ) -> Option<Chain> {
        let line = rig.line(from);
        let rest = rig.rest_poses();
        let mut posed = rest.clone();
        let view = SkeletonView {
            parents: &rig.parents,
            rest: &rest,
            local: &mut posed,
            root_global: glam::Affine3A::IDENTITY,
        };
        let chain = view.chain(*line.first()?, *line.last()?).ok()?;
        Some(Chain {
            chain,
            follow,
            drag,
            stiffness,
            gravity,
            spring: Box::new(SpringBoneSettings {
                stiffness,
                drag,
                gravity,
                ..SpringBoneSettings::default()
            }),
            state: Box::default(),
            trail: Vec::new(),
            stood: None,
        })
    }

    pub(super) fn reset(&mut self) {
        self.state.reset();
        self.trail.clear();
        self.stood = None;
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        if self.follow {
            self.follow(frame);
        } else {
            self.spring(frame);
        }
    }

    /// Moves the spring on through the frame in even steps near `SPRING_STEP`, each
    /// from the pose the body's clips gave the chain this frame, with the body carried through the
    /// steps from where it stood to where it stands: a spring that met all of a frame's travel in
    /// its first step would take it as a jolt.
    fn spring(&mut self, frame: &mut Frame) {
        let stands = frame.view.root_global;
        let kept = self
            .stood
            .map(|stood| stood.steps)
            .filter(|&steps| (frame.dt / steps as f32 - SPRING_STEP).abs() <= SPRING_STEP / 2.0);
        let steps = kept.unwrap_or((frame.dt / SPRING_STEP).round().max(1.0) as u32);
        let step = frame.dt / steps as f32;
        let stood = self.stood.unwrap_or(Stood {
            at: stands,
            steps,
            step,
        });
        let mut last = stood.step;
        let joints = self.chain.as_slice().to_vec();
        let posed: Vec<BonePose> = joints
            .iter()
            .map(|&joint| frame.view.local[joint as usize])
            .collect();
        for nth in 0..steps {
            if nth > 0 {
                for (&joint, pose) in joints.iter().zip(&posed) {
                    frame.view.local[joint as usize] = *pose;
                }
            }
            self.pace(step, last);
            frame.view.root_global = carried(stood.at, stands, (nth + 1) as f32 / steps as f32);
            process_spring_bones(
                &mut frame.view,
                &self.chain,
                &self.spring,
                &mut self.state,
                &[],
                Vec3::ZERO,
                step,
            );
            last = step;
        }
        self.stood = Some(Stood {
            at: stands,
            steps,
            step,
        });
    }

    /// Gives the spring what it loses and is pulled by over a step of `step` after one of `last`.
    /// The spring carries its tail on by as far as it went the step before, and pulls it by as
    /// much each step however long the step is: so a step's carry is scaled to how much longer it
    /// is than the last, and its pull to its length and the last's, as time-corrected Verlet
    /// does. Otherwise, as frames take one step and then two, the swing would halve and double
    /// and the chain jitter.
    fn pace(&mut self, step: f32, last: f32) {
        let kept = (1.0 - self.drag).powf(step / SPRING_STEP) * step / last;
        let pulled = (step + last) / 2.0 / SPRING_STEP;
        self.spring.drag = 1.0 - kept;
        self.spring.stiffness = self.stiffness * pulled;
        self.spring.gravity = self.gravity * pulled;
    }

    /// Lays every joint after the first on the path the first went, as far along it as the chain
    /// is long to that joint.
    fn follow(&mut self, frame: &mut Frame) {
        let joints = self.chain.as_slice();
        let world =
            |view: &SkeletonView, joint: u16| Vec3::from(view.global_pose(joint).translation);
        let head = world(&frame.view, joints[0]);
        if self.trail.is_empty() {
            self.trail = joints
                .iter()
                .map(|&joint| world(&frame.view, joint))
                .collect();
        } else if self.trail[0].distance(head) > TRAIL_STEP {
            self.trail.insert(0, head);
        } else {
            self.trail[0] = head;
        }
        let mut along = 0.0;
        for pair in joints.windows(2) {
            let (joint, next) = (pair[0], pair[1]);
            along += frame.view.local[next as usize].translation.length()
                * frame.view.root_global.matrix3.y_axis.length();
            let Some(wanted) = at_distance(&self.trail, along) else {
                break;
            };
            let from = world(&frame.view, joint);
            let now = world(&frame.view, next) - from;
            let turn = Quat::from_rotation_arc(
                now.normalize_or_zero(),
                (wanted - from).normalize_or_zero(),
            );
            let (_, rotation, _) = frame
                .view
                .global_pose(joint)
                .to_scale_rotation_translation();
            frame.view.set_global_rotation(joint, turn * rotation);
        }
        let total = along;
        let mut walked = 0.0;
        let mut keep = self.trail.len();
        for (index, pair) in self.trail.windows(2).enumerate() {
            walked += pair[0].distance(pair[1]);
            if walked > total + TRAIL_STEP {
                keep = index + 2;
                break;
            }
        }
        self.trail.truncate(keep);
    }
}

/// Where a body that stood at `from` and stands at `to` is `share` of the way between them.
fn carried(from: Affine3A, to: Affine3A, share: f32) -> Affine3A {
    let (from_scale, from_turn, from_at) = from.to_scale_rotation_translation();
    let (to_scale, to_turn, to_at) = to.to_scale_rotation_translation();
    Affine3A::from_scale_rotation_translation(
        from_scale.lerp(to_scale, share),
        from_turn.slerp(to_turn, share),
        from_at.lerp(to_at, share),
    )
}

/// The point `distance` along the path `trail` from its first point, if it is that long.
fn at_distance(trail: &[Vec3], distance: f32) -> Option<Vec3> {
    let mut walked = 0.0;
    for pair in trail.windows(2) {
        let length = pair[0].distance(pair[1]);
        if walked + length >= distance {
            let t = if length > 0.0 {
                (distance - walked) / length
            } else {
                0.0
            };
            return Some(pair[0].lerp(pair[1], t));
        }
        walked += length;
    }
    None
}
