//! A chain of joints behind the body: sprung, each joint's tail carried on by its momentum and
//! pulled back toward its pose, as VRM spring bones are; or following, each joint trailing the one
//! before it along the path the first went.

use bevy_skeleton_modifiers::core::{
    ChainIndices, SkeletonView, SpringBoneSettings, SpringBoneState, process_spring_bones,
};
use glam::{Quat, Vec3};

use super::{Frame, Rig};

/// How far the first joint moves before the path it went takes another point.
const TRAIL_STEP: f32 = 0.02;

pub struct Chain {
    chain: ChainIndices,
    follow: bool,
    spring: Box<SpringBoneSettings>,
    state: Box<SpringBoneState>,
    /// Where the first joint has been in the world, the latest first.
    trail: Vec<Vec3>,
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
            spring: Box::new(SpringBoneSettings {
                stiffness,
                drag,
                gravity,
                ..SpringBoneSettings::default()
            }),
            state: Box::default(),
            trail: Vec::new(),
        })
    }

    pub(super) fn reset(&mut self) {
        self.state.reset();
        self.trail.clear();
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        if self.follow {
            self.follow(frame);
        } else {
            process_spring_bones(
                &mut frame.view,
                &self.chain,
                &self.spring,
                &mut self.state,
                &[],
                Vec3::ZERO,
                frame.dt,
            );
        }
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
