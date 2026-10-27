//! Joints held at turns of their own over whatever the clips play, eased in as the hold begins.

use glam::Quat;

use super::Frame;

pub struct HeldPose {
    joints: Vec<(u16, Quat)>,
    ease: f32,
    shown: f32,
}

impl HeldPose {
    /// Holds each joint at its turn, easing in over `ease` seconds.
    pub fn new(joints: Vec<(u16, Quat)>, ease: f32) -> HeldPose {
        HeldPose {
            joints,
            ease: ease.max(f32::EPSILON),
            shown: 0.0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.shown = 0.0;
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        let fade = (-frame.dt / self.ease).exp();
        self.shown += (1.0 - self.shown) * (1.0 - fade);
        for &(joint, turn) in &self.joints {
            if let Some(pose) = frame.view.local.get_mut(joint as usize) {
                pose.rotation = pose.rotation.slerp(turn, self.shown);
            }
        }
    }
}
