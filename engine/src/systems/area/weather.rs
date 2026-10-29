use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use bevy::prelude::*;

use crate::core::content::{Content, RowCheck, serial};
use crate::core::geometry::Degrees;
use crate::core::render::world::sky::Daylight;
use crate::core::time::{DayOrigin, Seconds, UnixMillis, server_now};
use crate::systems::GameSettings;

/// The weather an area stands in, fixed for the area: its wind, how much cloud covers its sky
/// and how much water the cloud holds. What grows and hangs in the area sways with the wind, the
/// clouds drift with it and cast their shadows, the sky and the light dim under them, the more so
/// the wetter they are, and shaders read all of it through the engine.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    #[serde(default, skip_serializing_if = "Wind::is_calm")]
    pub wind: Wind,
    /// 0 no cloud, 1 wholly clouded over.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub clouds: f32,
    /// 0 soft white fair-weather cloud, 1 dark storm cloud.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub water: f32,
}

/// A game's day and night: a world starts at `start` o'clock and the whole cycle turns once every
/// `day_length` of real time from then on.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayCycle {
    pub day_length: Seconds,
    pub start: f32,
}

impl DayCycle {
    /// When the day began for a world starting `now`.
    pub fn origin(&self, now: UnixMillis) -> UnixMillis {
        now.before(Seconds(self.start / 24.0 * self.day_length.0))
    }

    /// The hour of the day, 0 to 24, `since` the day began.
    pub fn hour_at(&self, since: Seconds) -> f32 {
        since.0.rem_euclid(self.day_length.0) / self.day_length.0 * 24.0
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/day_length"),
            self.day_length.0,
            60.0,
            86_400.0,
            "real seconds a day lasts",
        );
        cx.range(format!("{pointer}/start"), self.start, 0.0, 24.0, "o'clock");
    }
}

/// Keeps the client's daylight at the game's hour, or fixed for a game without a day cycle or
/// before the server has told the time.
pub fn tell_time(world: &mut World) {
    let hour = || {
        let cycle = world
            .resource::<Content>()
            .built::<GameSettings>()
            .daylight?;
        let origin = world.resource::<DayOrigin>().0?;
        Some(cycle.hour_at(server_now(world)?.since(origin)))
    };
    let now = hour().map_or(Daylight::Fixed, Daylight::Hour);
    world.resource_mut::<Daylight>().set_if_neq(now);
}

/// Wind blowing toward `direction` (degrees from north, clockwise) at `strength`, from calm at 0
/// to a gale at 1.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wind {
    pub direction: Degrees,
    pub strength: f32,
}

impl Weather {
    pub const STILL: Weather = Weather {
        wind: Wind::CALM,
        clouds: 0.0,
        water: 0.0,
    };

    /// A light breeze from the south-west under a clear sky, as a day outside has.
    pub fn breezy() -> Weather {
        Weather {
            wind: Wind {
                direction: Degrees(240.0),
                strength: 0.35,
            },
            clouds: 0.0,
            water: 0.0,
        }
    }

    pub fn is_still(&self) -> bool {
        self.wind.is_calm() && self.clouds == 0.0 && self.water == 0.0
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/wind/direction"),
            self.wind.direction.0,
            0.0,
            360.0,
            "degrees from north",
        );
        cx.range(
            format!("{pointer}/wind/strength"),
            self.wind.strength,
            0.0,
            1.0,
            "of a gale",
        );
        cx.range(
            format!("{pointer}/clouds"),
            self.clouds,
            0.0,
            1.0,
            "of the sky clouded over",
        );
        cx.range(
            format!("{pointer}/water"),
            self.water,
            0.0,
            1.0,
            "of the way from fair-weather cloud to storm cloud",
        );
    }
}

impl Wind {
    pub const CALM: Wind = Wind {
        direction: Degrees(0.0),
        strength: 0.0,
    };

    pub fn is_calm(&self) -> bool {
        self.strength <= 0.0
    }
}
