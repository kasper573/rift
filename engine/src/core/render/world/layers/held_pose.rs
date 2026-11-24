//! Joints held at turns of their own over whatever the clips play, eased in as the hold begins
//! and out as it ends.

use glam::Quat;

use super::Frame;
use super::blend::Blend;

pub struct HeldPose {
    joints: Vec<(u16, Quat)>,
    blend: Blend,
}

impl HeldPose {
    /// Holds each joint at its turn, easing in and out over `ease` seconds.
    pub fn new(joints: Vec<(u16, Quat)>, ease: f32) -> HeldPose {
        HeldPose {
            joints,
            blend: Blend::new(ease),
        }
    }

    pub(super) fn reset(&mut self) {
        self.blend.reset();
    }

    pub(super) fn resting(&self) -> bool {
        self.blend.resting()
    }

    /// Eases the hold in while `on`, and out of it while not.
    pub(super) fn run(&mut self, frame: &mut Frame, on: bool) {
        self.blend.run(frame.dt, on);
        let shown = self.blend.shown();
        for &(joint, turn) in &self.joints {
            if let Some(pose) = frame.view.local.get_mut(joint as usize) {
                pose.rotation = pose.rotation.slerp(turn, shown);
            }
        }
    }
}
