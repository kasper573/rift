//! How much of a layer shows as it comes and goes: from none to all over its ease, at an even
//! pace eased at both ends, and none again as surely when it goes.

use crate::core::math::smoothstep;

pub struct Blend {
    over: f32,
    through: f32,
}

impl Blend {
    /// A blend coming and going over `over` seconds; at once over none.
    pub fn new(over: f32) -> Blend {
        Blend { over, through: 0.0 }
    }

    /// Moves the blend on `dt` seconds toward all while `on`, and toward none while not.
    pub fn run(&mut self, dt: f32, on: bool) {
        let wanted = if on { 1.0 } else { 0.0 };
        let pace = if self.over > 0.0 { dt / self.over } else { 1.0 };
        self.through += (wanted - self.through).clamp(-pace, pace);
    }

    /// How much of the layer shows, from none to all.
    pub fn shown(&self) -> f32 {
        smoothstep(0.0, 1.0, self.through)
    }

    pub fn resting(&self) -> bool {
        self.through == 0.0
    }

    pub fn reset(&mut self) {
        self.through = 0.0;
    }
}
