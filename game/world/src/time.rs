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

impl Millis {
    pub fn seconds(self) -> Seconds {
        Seconds(self.0 / 1000.0)
    }
}
