//! Temporal units and rates.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Seconds(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Millis(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct PlaybackRate(pub f32);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Hertz(pub f32);

impl PlaybackRate {
    pub fn at_least(self, floor: f32) -> PlaybackRate {
        PlaybackRate(self.0.max(floor))
    }
}

impl Hertz {
    pub fn period(self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.0)
    }
}

impl std::ops::Add for Seconds {
    type Output = Seconds;
    fn add(self, other: Seconds) -> Seconds {
        Seconds(self.0 + other.0)
    }
}

impl std::ops::Sub for Seconds {
    type Output = Seconds;
    fn sub(self, other: Seconds) -> Seconds {
        Seconds(self.0 - other.0)
    }
}

impl std::ops::Div<PlaybackRate> for Seconds {
    type Output = Seconds;
    fn div(self, rate: PlaybackRate) -> Seconds {
        Seconds(self.0 / rate.0)
    }
}

impl std::ops::Add for Millis {
    type Output = Millis;
    fn add(self, other: Millis) -> Millis {
        Millis(self.0 + other.0)
    }
}

impl std::ops::AddAssign for Millis {
    fn add_assign(&mut self, other: Millis) {
        self.0 += other.0;
    }
}

impl std::ops::Sub for Millis {
    type Output = Millis;
    fn sub(self, other: Millis) -> Millis {
        Millis(self.0 - other.0)
    }
}

impl std::ops::SubAssign for Millis {
    fn sub_assign(&mut self, other: Millis) {
        self.0 -= other.0;
    }
}

impl std::ops::Mul<PlaybackRate> for Millis {
    type Output = Millis;
    fn mul(self, rate: PlaybackRate) -> Millis {
        Millis(self.0 * rate.0)
    }
}

impl std::ops::Rem for Millis {
    type Output = Millis;
    fn rem(self, other: Millis) -> Millis {
        Millis(self.0 % other.0)
    }
}

impl Seconds {
    pub fn millis(self) -> Millis {
        Millis(self.0 * 1000.0)
    }
}

impl Millis {
    pub fn seconds(self) -> Seconds {
        Seconds(self.0 / 1000.0)
    }

    pub fn min(self, other: Millis) -> Millis {
        Millis(self.0.min(other.0))
    }

    pub fn max(self, other: Millis) -> Millis {
        Millis(self.0.max(other.0))
    }

    pub fn ratio(self, other: Millis) -> f32 {
        self.0 / other.0
    }
}
