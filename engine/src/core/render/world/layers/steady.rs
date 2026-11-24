//! A hand held out steady: kept at a point off the chest whatever the clips swing its arm
//! through, reached by turning the upper arm and the forearm, the point followed on a spring so
//! the hand lags and swings a little as the chest moves.

use bevy_skeleton_modifiers::core::{TwoBoneSettings, process_two_bone};
use glam::Vec3;

use super::Frame;
use super::blend::Blend;

/// The longest step the spring is moved on by, so it swings alike at any frame rate.
const SPRING_STEP: f32 = 1.0 / 120.0;

/// What a hand holds steady: where it rests off the chest, in metres in the body's frame (out to
/// the hand's side, up, ahead), and the spring it follows that point on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteadyHand {
    pub at: Vec3,
    pub hz: f32,
    pub damping: f32,
}

pub struct Steady {
    chest: u16,
    upper_arm: u16,
    forearm: u16,
    hand: u16,
    blend: Blend,
    held: Option<SteadyHand>,
    /// Where the spring holds the hand and how fast it moves, in the body's frame.
    spring: Option<(Vec3, Vec3)>,
}

impl Steady {
    /// The hand at `hand`, under `forearm` and `upper_arm`, held off `chest`, easing in and out
    /// over `ease` seconds.
    pub fn new(chest: u16, upper_arm: u16, forearm: u16, hand: u16, ease: f32) -> Steady {
        Steady {
            chest,
            upper_arm,
            forearm,
            hand,
            blend: Blend::new(ease),
            held: None,
            spring: None,
        }
    }

    /// Holds the hand as `held` says; while it holds nothing steady the last hold eases out.
    pub fn hold(&mut self, held: Option<SteadyHand>) {
        if held.is_some() {
            self.held = held;
        }
    }

    pub(super) fn resting(&self) -> bool {
        self.blend.resting()
    }

    pub(super) fn reset(&mut self) {
        self.blend.reset();
        self.spring = None;
    }

    pub(super) fn run(&mut self, frame: &mut Frame, on: bool) {
        self.blend.run(frame.dt, on);
        let Some(held) = self.held.filter(|_| !self.blend.resting()) else {
            self.reset();
            return;
        };
        let shown = self.blend.shown();
        let view = &frame.view;
        let units = view.root_global.matrix3.y_axis.length().max(f32::EPSILON);
        let chest = Vec3::from(view.skeleton_pose(self.chest).translation);
        let shoulder = Vec3::from(view.skeleton_pose(self.upper_arm).translation);
        let elbow = Vec3::from(view.skeleton_pose(self.forearm).translation);
        let hand = Vec3::from(view.skeleton_pose(self.hand).translation);
        let side = if shoulder.x < chest.x { -1.0 } else { 1.0 };
        let rest = chest + Vec3::new(held.at.x * side, held.at.y, held.at.z) / units;
        let target = self.follow(rest, held, frame.dt);
        let pole = elbow + (elbow - (shoulder + hand) * 0.5);
        let before = [self.upper_arm, self.forearm].map(|joint| frame.view.local[joint as usize]);
        process_two_bone(
            &mut frame.view,
            self.upper_arm,
            self.forearm,
            self.hand,
            target,
            Some(pole),
            &TwoBoneSettings::default(),
        );
        for (joint, before) in [self.upper_arm, self.forearm].into_iter().zip(before) {
            let solved = &mut frame.view.local[joint as usize];
            solved.rotation = before.rotation.slerp(solved.rotation, shown);
        }
    }

    /// Moves the spring on toward `rest` over `dt` and gives where it holds the hand now.
    fn follow(&mut self, rest: Vec3, held: SteadyHand, dt: f32) -> Vec3 {
        let (mut at, mut speed) = self.spring.unwrap_or((rest, Vec3::ZERO));
        let swing = std::f32::consts::TAU * held.hz;
        let steps = (dt / SPRING_STEP).ceil().max(1.0);
        let step = dt / steps;
        for _ in 0..steps as usize {
            let pull = (rest - at) * swing * swing - speed * (2.0 * held.damping * swing);
            speed += pull * step;
            at += speed * step;
        }
        self.spring = Some((at, speed));
        at
    }
}
