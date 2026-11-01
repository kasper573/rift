//! A joint squashed down and spread out by the body's speed and on every step, its volume kept,
//! easing in and out.

use glam::Vec3;

use super::Frame;

pub struct Squash {
    joint: u16,
    by_speed: f32,
    on_step: f32,
    ease: f32,
    most: f32,
    landing: f32,
    shown: f32,
}

impl Squash {
    /// Squashes `joint` by `by_speed` for every metre a second the body goes and by `on_step` as a
    /// foot comes down, easing over `ease` seconds, at most by `most` of its height.
    pub fn new(joint: u16, by_speed: f32, on_step: f32, ease: f32, most: f32) -> Squash {
        Squash {
            joint,
            by_speed,
            on_step,
            ease: ease.max(f32::EPSILON),
            most,
            landing: 0.0,
            shown: 0.0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.landing = 0.0;
        self.shown = 0.0;
    }

    pub(super) fn run(&mut self, frame: &mut Frame) {
        if frame.stepped {
            self.landing = 1.0;
        }
        let fade = (-frame.dt / self.ease).exp();
        let wanted = self.by_speed * frame.speed + self.on_step * self.landing;
        self.landing *= fade;
        self.shown += (wanted - self.shown) * (1.0 - fade);
        let tall = 1.0 - self.shown.clamp(0.0, self.most);
        let wide = 1.0 / tall.sqrt();
        if let Some(pose) = frame.view.local.get_mut(self.joint as usize) {
            pose.scale *= Vec3::new(wide, tall, wide);
        }
    }
}
