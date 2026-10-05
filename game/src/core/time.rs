use bevy_ecs::prelude::Resource;
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
    derive_more::SubAssign,
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

    pub fn min(self, other: Seconds) -> Seconds {
        Seconds(self.0.min(other.0))
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

#[derive(Resource, Clone, Copy, Debug)]
pub struct WallClock {
    pub now: UnixMillis,
    pub reset: UtcHour,
}

impl WallClock {
    pub fn day(self) -> GameDay {
        GameDay::of(self.now, self.reset)
    }

    pub fn until_next_day(self) -> Seconds {
        let next =
            UnixMillis((self.day().0 + 1) * DAY_MILLIS + u64::from(self.reset.0) * HOUR_MILLIS);
        next.since(self.now)
    }
}

const HOUR_MILLIS: u64 = 3_600_000;
const DAY_MILLIS: u64 = 24 * HOUR_MILLIS;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixMillis(pub u64);

impl UnixMillis {
    pub fn after(self, duration: Seconds) -> UnixMillis {
        UnixMillis(self.0.saturating_add(duration.millis().0.max(0.0) as u64))
    }

    pub fn since(self, earlier: UnixMillis) -> Seconds {
        Millis(self.0.saturating_sub(earlier.0) as f32).seconds()
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(try_from = "u8")]
pub struct UtcHour(u8);

impl UtcHour {
    pub const MIDNIGHT: UtcHour = UtcHour(0);
}

impl TryFrom<u8> for UtcHour {
    type Error = String;

    fn try_from(hour: u8) -> Result<UtcHour, String> {
        if hour < 24 {
            Ok(UtcHour(hour))
        } else {
            Err(format!("a UTC hour is 0 to 23, got {hour}"))
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameDay(pub u64);

impl GameDay {
    pub fn of(now: UnixMillis, reset: UtcHour) -> GameDay {
        GameDay(now.0.saturating_sub(u64::from(reset.0) * HOUR_MILLIS) / DAY_MILLIS)
    }
}
