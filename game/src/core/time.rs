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

    pub fn in_words(self) -> String {
        let exact = self.0.ceil().max(1.0) as u32;
        let (_, small) = unit_pair(exact);
        let rounded = exact.div_ceil(small.size) * small.size;
        let (big, small) = unit_pair(rounded);
        [
            (rounded / big.size, big.name),
            (rounded % big.size / small.size, small.name),
        ]
        .into_iter()
        .filter(|&(count, _)| count > 0)
        .map(|(count, name)| match count {
            1 => format!("1 {name}"),
            _ => format!("{count} {name}s"),
        })
        .collect::<Vec<_>>()
        .join(" and ")
    }
}

impl From<Seconds> for std::time::Duration {
    fn from(seconds: Seconds) -> std::time::Duration {
        std::time::Duration::from_secs_f32(seconds.0.max(0.0))
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

#[derive(Clone, Copy, Debug)]
pub struct LocalClock {
    pub now: UnixMillis,
    pub utc_offset: Seconds,
}

impl LocalClock {
    pub fn label(self, at: UnixMillis) -> String {
        let local = |time: UnixMillis| {
            let millis = time.0 as i64 + self.utc_offset.millis().0 as i64;
            (
                millis.div_euclid(DAY_MILLIS as i64),
                millis.rem_euclid(DAY_MILLIS as i64),
            )
        };
        let (day, of_day) = local(at);
        let (today, _) = local(self.now);
        let minutes = of_day / 60_000;
        let clock = format!("{:02}:{:02}", minutes / 60, minutes % 60);
        match day == today {
            true => clock,
            false => format!("{} {clock}", WEEKDAYS[day.rem_euclid(7) as usize]),
        }
    }
}

const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

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

#[derive(Clone, Copy)]
struct TimeUnit {
    size: u32,
    name: &'static str,
}

const TIME_UNITS: [TimeUnit; 4] = [
    TimeUnit {
        size: 86_400,
        name: "day",
    },
    TimeUnit {
        size: 3_600,
        name: "hour",
    },
    TimeUnit {
        size: 60,
        name: "minute",
    },
    TimeUnit {
        size: 1,
        name: "second",
    },
];

fn unit_pair(seconds: u32) -> (TimeUnit, TimeUnit) {
    let big = TIME_UNITS
        .iter()
        .position(|unit| seconds >= unit.size)
        .unwrap_or(TIME_UNITS.len() - 1)
        .min(TIME_UNITS.len() - 2);
    (TIME_UNITS[big], TIME_UNITS[big + 1])
}
