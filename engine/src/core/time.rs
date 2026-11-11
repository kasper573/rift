use std::hash::Hash;
use std::time::SystemTime;

use bevy_ecs::prelude::{Resource, World};
use bevy_time::{Real, Time};
use serde::{Deserialize, Serialize};

use crate::core::math::Rng;
use crate::core::units::{Millis, Seconds};

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

    pub fn before(self, duration: Seconds) -> UnixMillis {
        UnixMillis(self.0.saturating_sub(duration.millis().0.max(0.0) as u64))
    }

    pub fn since(self, earlier: UnixMillis) -> Seconds {
        Millis(self.0.saturating_sub(earlier.0) as f32).seconds()
    }
}

impl From<SystemTime> for UnixMillis {
    fn from(time: SystemTime) -> UnixMillis {
        let since_epoch = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("the clock is set after 1970");
        UnixMillis(since_epoch.as_millis() as u64)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LocalClock {
    pub now: UnixMillis,
    pub utc_offset: Seconds,
}

impl LocalClock {
    /// This machine's clock, in its time zone.
    pub fn of_this_machine() -> LocalClock {
        LocalClock {
            now: std::time::SystemTime::now().into(),
            utc_offset: Seconds(chrono::Local::now().offset().local_minus_utc() as f32),
        }
    }

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
        if day == today {
            clock
        } else {
            format!("{} {clock}", WEEKDAYS[day.rem_euclid(7) as usize])
        }
    }
}

// Day 0, the Unix epoch, was a Thursday.
const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    pub first: Seconds,
    pub longest: Seconds,
}

impl Backoff {
    pub fn after(self, failures: u32, rng: &mut Rng) -> Seconds {
        let doubled = self.first * 2f32.powi(failures.saturating_sub(1).min(16) as i32);
        // Jittered, so clients that failed together don't all try again together.
        doubled.min(self.longest) * (0.5 + rng.rand_float())
    }
}

/// The server's wall clock as the welcome carried it, against how long the client had really run
/// by then, so the client can tell how far along a server deadline is without a wall clock of its
/// own.
#[derive(Resource, Default)]
pub struct ServerClock(Option<(UnixMillis, Seconds)>);

impl ServerClock {
    pub fn record(&mut self, then: UnixMillis, uptime: Seconds) {
        self.0 = Some((then, uptime));
    }

    pub fn now(&self, uptime: Seconds) -> Option<UnixMillis> {
        let (then, at) = self.0?;
        Some(then.after(uptime - at))
    }
}

/// How the game's day runs, as the server keeps it, or nothing for a game without a day cycle:
/// turning, or held at an hour of the day. The server sets it as it starts and tells every client
/// on joining, so all of them read the same hour off their reckoning of the server's clock.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct DayClock(pub Option<TimeCourse<f32>>);

/// How the weather's course runs on the server the client plays on: since the server started, or
/// held at one of each area's spells, by its place among them; none before the server has told it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherClock(pub Option<TimeCourse<usize>>);

/// How a game's own time runs on its server: turning since a moment, or held at a point along
/// it, as tests and films that compare how the world looks need it to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum TimeCourse<Held> {
    /// Turning since the moment it began.
    Since(UnixMillis),
    /// Held at a point along it.
    Held(Held),
}

/// The server's wall clock as this client reckons it, once the server has told it the time. It
/// is reckoned by the real time that has passed, which a slow frame does not hold back as it does
/// the world's own clock.
pub fn server_now(world: &World) -> Option<UnixMillis> {
    world
        .get_resource::<ServerClock>()?
        .now(real_uptime(world.get_resource::<Time<Real>>()?))
}

/// How long the program has really run.
pub fn real_uptime(time: &Time<Real>) -> Seconds {
    Seconds(time.elapsed_secs())
}

/// How long the world has run, by its own clock.
pub fn uptime(world: &World) -> Seconds {
    Seconds(world.resource::<Time>().elapsed_secs())
}
