use serde::{Deserialize, Serialize};

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::AddAssign,
    derive_more::Sub,
)]
pub struct Seconds(pub f32);

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::AddAssign,
    derive_more::Sub,
    derive_more::SubAssign,
)]
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

impl PlaybackRate {
    pub fn at_least(self, floor: f32) -> PlaybackRate {
        PlaybackRate(self.0.max(floor))
    }
}

impl std::ops::Div<PlaybackRate> for Seconds {
    type Output = Seconds;
    fn div(self, rate: PlaybackRate) -> Seconds {
        Seconds(self.0 / rate.0)
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

    pub fn ratio(self, other: Seconds) -> f32 {
        self.0 / other.0
    }
}

impl std::ops::Mul<f32> for Seconds {
    type Output = Seconds;
    fn mul(self, factor: f32) -> Seconds {
        Seconds(self.0 * factor)
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
