//! A wave down a chain of joints: each joint turned about the body's up by a sine that lags the
//! one before, growing toward the chain's end.

use glam::Quat;

use super::{Frame, Rig};

pub struct Wave {
    joints: Vec<u16>,
    amplitude: f32,
    hz: f32,
    lag: f32,
}

impl Wave {
    /// A wave from `from` down the rig, turning its last joint by up to `amplitude` radians `hz`
    /// times a second, each joint `lag` of a turn behind the one before.
    pub fn new(rig: &Rig, from: u16, amplitude: f32, hz: f32, lag: f32) -> Wave {
        Wave {
            joints: rig.line(from),
            amplitude,
            hz,
            lag,
        }
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        let count = self.joints.len() as f32;
        let up = frame
            .view
            .root_global
            .matrix3
            .y_axis
            .normalize_or_zero()
            .into();
        for (index, &joint) in self.joints.iter().enumerate() {
            let phase = std::f32::consts::TAU * (self.hz * frame.now - self.lag * index as f32);
            let angle = self.amplitude * (index + 1) as f32 / count * phase.sin() / count;
            let (_, rotation, _) = frame
                .view
                .global_pose(joint)
                .to_scale_rotation_translation();
            frame
                .view
                .set_global_rotation(joint, Quat::from_axis_angle(up, angle) * rotation);
        }
    }
}
