use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use bevy::prelude::*;

use crate::core::audio::playback::SfxId;
use crate::core::content::{
    AudioRef, Content, ContentProblem, ImageRef, RowCheck, SchemaMark, nearest, serial,
};
use crate::core::math::{Rgb, Rng};
use crate::core::render::world::clouds::CloudDrift;
use crate::core::render::world::sky::Daylight;
use crate::core::time::{
    self, DayClock, TimeCourse, UnixMillis, WallClock, WeatherClock, server_now,
};
use crate::core::units::{DayHours, Degrees, HourOfDay, Meters, Seconds};
use crate::core::vfx::rain::NEAREST_SHELL;
use crate::core::vfx::{ParamValue, VfxShaderId};
use crate::systems::area::AreaId;
use crate::systems::launch::{ServerDay, ServerDaylight};
use crate::systems::script::sky::{self, Sky};
use crate::systems::{GameSettings, WorldArea};

/// How many shells rain is drawn on round the eye at most, each over the whole picture.
pub const MAX_RAIN_SHELLS: usize = 6;

/// The weather of a spell: its wind, how much cloud covers the sky, how much water the cloud
/// holds, how hard it rains under the cloud that holds water and how often lightning strikes. What
/// grows and hangs in the area sways with the wind, the clouds drift with it and cast their
/// shadows, the sky and the light dim under them, the more so the wetter they are, rain falls
/// where cloud holding water drifts over, and shaders read all of it through the engine.
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
    /// 0 no rain, 1 a cloudburst, under the cloud that holds the most water.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub rain: f32,
    /// How many times a minute lightning strikes somewhere in sight, thunder rolling after.
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub lightning: f32,
}

/// The weather an area turns through: spells of weather, each lasting a while and giving way, over
/// `turns`, to one of the spells that may follow it, by their weights. Its course starts at the
/// first spell as the server starts and runs alike on every client, the course `seed` sets; a
/// server that holds the weather still keeps the first spell. Its rain falls as `rain` says, and
/// lightning strikes as `lightning` does; without them, no rain is seen and no lightning.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Climate {
    pub spells: Vec<Spell>,
    pub turns: Seconds,
    #[serde(default, skip_serializing_if = "serial::is_zero_u64")]
    pub seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rain: Option<Rainfall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightning: Option<Lightning>,
}

/// How an area's rain falls: from the clouds that hold water, carried `slant` metres downwind for
/// every metre it falls in a full wind, so it lands downwind of the cloud it falls from; drawn
/// round the eye by a shader of the rain stage, filled with `params`, on a shell at each distance
/// of `shells`; the ground soaking through over `wets` of a cloudburst and drying over `dries` once
/// the rain stops; and sounding as `sounds` say.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rainfall {
    pub shader: VfxShaderId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(transform = SchemaMark::ParamsOf.with("shader"))]
    pub params: BTreeMap<String, ParamValue>,
    pub shells: Vec<Meters>,
    pub slant: f32,
    pub wets: Seconds,
    pub dries: Seconds,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<RainSound>,
}

/// How lightning strikes: lighting the world, for a moment, `flash` times as brightly as the open
/// sky lights it by day where it strikes within its thunder's `near`, fainter the further off, its
/// thunder following.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lightning {
    pub flash: f32,
    pub thunder: Thunder,
}

/// The thunder of a strike, following its flash at the speed of sound: the `clap` of a strike
/// close by, sharp and soon lost to the air, and the `roll` that carries. Both play as loud as
/// their clips for a strike within `near`. Further off the clap falls away with the square of the
/// distance and the roll with its root, fading to nothing at `reach`.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thunder {
    pub clap: SfxId,
    pub roll: SfxId,
    pub near: Meters,
    pub reach: Meters,
}

/// How the thunder of one strike is heard: how loud its clap and its roll, 0 silent to 1 as loud
/// as their clips, and how long after the flash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThunderHeard {
    pub clap: f32,
    pub roll: f32,
    pub after: Seconds,
}

impl Thunder {
    /// How fast thunder travels, in metres a second.
    const SPEED: f32 = 343.0;

    /// How near a strike `away` is: 1 within `near`, falling away with the distance.
    pub fn nearness(&self, away: Meters) -> f32 {
        (self.near.0 / away.0.max(f32::EPSILON)).min(1.0)
    }

    pub fn heard(&self, away: Meters) -> ThunderHeard {
        let nearness = self.nearness(away);
        let carried = (1.0 - (away.0 - self.near.0) / (self.reach.0 - self.near.0)).clamp(0.0, 1.0);
        ThunderHeard {
            clap: nearness * nearness,
            roll: nearness.sqrt() * carried,
            after: Seconds(away.0 / Thunder::SPEED),
        }
    }
}

/// A spell of weather: what it is called, its weather, how long it lasts at the shortest and the
/// longest, and the spells that may follow it, each by its name with its weight; with none, it
/// holds for good.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spell {
    pub name: String,
    pub weather: Weather,
    pub lasts: SpellLength,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<NextSpell>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellLength {
    pub shortest: Seconds,
    pub longest: Seconds,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NextSpell {
    pub spell: String,
    pub weight: f32,
}

/// A sound of rain, on its soundscape `channel`: loudest where it rains `rain` hard, 0 to 1,
/// fading out toward the next sound's rain either way, at `volume`, and silent where no rain falls.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RainSound {
    pub channel: u32,
    pub file: AudioRef,
    pub rain: f32,
    #[schemars(extend("default" = serial::one_f32()))]
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub volume: f32,
}

/// Where a climate's course stands: the spell under way, which one follows it, when it began and
/// when it gives way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpellCourse {
    step: u64,
    spell: usize,
    next: usize,
    began: UnixMillis,
    ends: UnixMillis,
}

/// A game's day and night: a world starts at `start` o'clock and the whole cycle turns once every
/// `day_length` of real time from then on, through its `times` in their order.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayCycle {
    pub day_length: Seconds,
    pub start: HourOfDay,
    /// The times the day is divided into, in the order they come. Without any the day has its
    /// clock and nothing can follow a time of it.
    #[schemars(transform = SchemaMark::DayTimes.with(true))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub times: Vec<TimeOfDay>,
}

/// A time of the game's day: it begins `from` o'clock and holds until the next of the day's times
/// begins, the last past midnight into the first. It comes over `eases`, half before `from` and
/// half after, as the one before it goes. Its `color` and `icon` are how the game shows it.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeOfDay {
    pub name: TimeOfDayName,
    pub from: HourOfDay,
    #[serde(default, skip_serializing_if = "DayHours::is_zero")]
    pub eases: DayHours,
    pub color: Rgb,
    pub icon: ImageRef,
}

/// One of the game's times of day, by its name.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[schemars(transform = SchemaMark::TimeOfDay.with(true))]
pub struct TimeOfDayName(pub String);

impl DayCycle {
    /// How the day runs for a world starting `now` on a server running it as `day` says.
    pub fn course(&self, day: ServerDay, now: UnixMillis) -> TimeCourse<HourOfDay> {
        match day.daylight {
            ServerDaylight::Turning => self.turning_from(self.start, now),
            ServerDaylight::Still => TimeCourse::Held(self.start),
            ServerDaylight::Fixed => {
                TimeCourse::Held(day.hour.expect("a fixed daylight names its hour"))
            }
        }
    }

    /// The day turning on from `hour` o'clock at `now`.
    pub fn turning_from(&self, hour: HourOfDay, now: UnixMillis) -> TimeCourse<HourOfDay> {
        TimeCourse::Since(now.before(Seconds(hour.share() * self.day_length.0)))
    }

    /// The hour of the day at `now` on a day running its `course`.
    pub fn hour(&self, course: TimeCourse<HourOfDay>, now: UnixMillis) -> HourOfDay {
        match course {
            TimeCourse::Since(origin) => HourOfDay(
                now.since(origin).0.rem_euclid(self.day_length.0) / self.day_length.0 * 24.0,
            ),
            TimeCourse::Held(hour) => hour,
        }
    }

    /// The time of day that holds at `hour`, when the game names any.
    pub fn time_at(&self, hour: HourOfDay) -> Option<&TimeOfDay> {
        self.holding(hour).map(|at| &self.times[at])
    }

    /// How much of the time named `time` has come at `hour`, 0 to 1; the shares of all the day's
    /// times add up to 1.
    pub fn share(&self, time: &TimeOfDayName, hour: HourOfDay) -> f32 {
        let Some(at) = self.holding(hour) else {
            return 0.0;
        };
        let count = self.times.len();
        let (before, after) = ((at + count - 1) % count, (at + 1) % count);
        let (holds, next) = (&self.times[at], &self.times[after]);
        let named = |index: usize| self.times[index].name == *time;
        if count == 1 {
            return if named(at) { 1.0 } else { 0.0 };
        }
        let come = HourOfDay::wrapped(hour.0).since(holds.from).0;
        let left = next.from.since(HourOfDay::wrapped(hour.0)).0;
        let (held, other) = if come < holds.eases.0 / 2.0 {
            (0.5 + come / holds.eases.0, before)
        } else if left < next.eases.0 / 2.0 {
            (0.5 + left / next.eases.0, after)
        } else {
            (1.0, at)
        };
        if named(at) {
            held
        } else if named(other) {
            1.0 - held
        } else {
            0.0
        }
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/day_length"),
            self.day_length.0,
            60.0,
            86_400.0,
            "real seconds a day lasts",
        );
        cx.range(
            format!("{pointer}/start"),
            self.start.0,
            0.0,
            24.0,
            "o'clock",
        );
        for (index, time) in self.times.iter().enumerate() {
            let at = format!("{pointer}/times/{index}");
            cx.text(format!("{at}/name"), &time.name.0, 1, 40);
            if !(0.0..24.0).contains(&time.from.0) {
                cx.error(
                    format!("{at}/from"),
                    "out of range",
                    time.from.0,
                    "o'clock 0 up to 24",
                );
            }
            if self.times[..index]
                .iter()
                .any(|other| other.name == time.name)
            {
                cx.error(
                    format!("{at}/name"),
                    "duplicate name",
                    &time.name.0,
                    "each time of day to have a name of its own",
                );
            }
            let earlier = &self.times[(index + self.times.len() - 1) % self.times.len()];
            if index > 0 && time.from == earlier.from {
                cx.error(
                    format!("{at}/from"),
                    "same hour",
                    format!(
                        "{} and {} both begin at {}",
                        earlier.name.0, time.name.0, time.from.0
                    ),
                    "each time of day to begin at its own hour",
                );
            } else if index > 0 && time.from < earlier.from {
                cx.error(
                    format!("{at}/from"),
                    "out of order",
                    format!(
                        "{} from {} after {} from {}",
                        time.name.0, time.from.0, earlier.name.0, earlier.from.0
                    ),
                    "the times of day in the order of their hours",
                );
            }
            cx.range(
                format!("{at}/eases"),
                time.eases.0,
                0.0,
                self.lasts(index).0.min(self.lasts_before(index).0),
                "hours, at most what the time and the one before it last",
            );
        }
    }

    /// Refuses a `name` that is none of the day's times, saying the nearest of them.
    pub fn check_named(
        daylight: Option<&DayCycle>,
        pointer: impl Into<String>,
        name: &TimeOfDayName,
        cx: &mut RowCheck,
    ) {
        if let Some(problem) = DayCycle::unknown_time(daylight, name) {
            cx.problem(pointer, problem);
        }
    }

    /// What is wrong with `name` when it is none of the day's times.
    pub fn unknown_time(
        daylight: Option<&DayCycle>,
        name: &TimeOfDayName,
    ) -> Option<ContentProblem> {
        let names: Vec<&String> = daylight
            .iter()
            .flat_map(|cycle| &cycle.times)
            .map(|time| &time.name.0)
            .collect();
        (!names.contains(&&name.0)).then(|| {
            ContentProblem::new(
                "unknown time of day",
                &name.0,
                "the name of one of the game's times of day",
            )
            .near(nearest(names, &name.0))
        })
    }

    fn holding(&self, hour: HourOfDay) -> Option<usize> {
        let hour = HourOfDay::wrapped(hour.0);
        self.times
            .iter()
            .rposition(|time| time.from <= hour)
            .or(self.times.len().checked_sub(1))
    }

    fn lasts(&self, index: usize) -> DayHours {
        let next = &self.times[(index + 1) % self.times.len()];
        match self.times.len() {
            1 => DayHours(24.0),
            _ => next.from.since(self.times[index].from),
        }
    }

    fn lasts_before(&self, index: usize) -> DayHours {
        self.lasts((index + self.times.len() - 1) % self.times.len())
    }
}

/// Keeps the client's daylight at the game's hour, or fixed for a game without a day cycle or
/// before the server has told the time.
pub fn tell_time(world: &mut World) {
    let hour = || {
        let cycle = world
            .resource::<Content>()
            .built::<GameSettings>()
            .daylight
            .as_ref()?;
        let course = world.resource::<DayClock>().0?;
        Some(cycle.hour(course, server_now(world)?))
    };
    let now = hour().map_or(Daylight::Fixed, Daylight::Hour);
    world.resource_mut::<Daylight>().set_if_neq(now);
}

/// Where the shown area's climate stands once it is known: the spell under way, and where along
/// the course it stands, so each frame walks on from the last; nowhere along it where the server
/// holds the weather at a spell.
#[derive(Resource, Default)]
pub struct ShownCourse(Option<Shown>);

#[derive(Clone, Copy)]
struct Shown {
    area: AreaId,
    spell: usize,
    along: Option<SpellCourse>,
}

impl ShownCourse {
    /// The spell under way over `area`, by its place among its climate's spells, once the course
    /// there is known.
    pub fn spell(&self, area: AreaId) -> Option<usize> {
        self.0
            .filter(|shown| shown.area == area)
            .map(|shown| shown.spell)
    }
}

/// The weather over the area `area`, of `climate`, at the server's moment on the course the server
/// runs its weather on; the first spell's before the server has told the time.
pub fn weather_now(world: &mut World, area: AreaId, climate: &Climate) -> Weather {
    let (Some(now), Some(course)) = (server_now(world), world.resource::<WeatherClock>().0) else {
        return climate.first();
    };
    let from = world
        .get_resource::<ShownCourse>()
        .and_then(|shown| shown.0)
        .filter(|shown| shown.area == area)
        .and_then(|shown| shown.along);
    let along = climate.course_at(course, now, from);
    let spell = match course {
        TimeCourse::Held(spell) => spell,
        TimeCourse::Since(_) => along.map_or(0, |along| along.spell()),
    };
    world.insert_resource(ShownCourse(Some(Shown { area, spell, along })));
    match course {
        TimeCourse::Held(spell) => climate.held(spell),
        TimeCourse::Since(_) => climate.weather_at(along, now),
    }
}

/// The sky over the area a world serves, as its server keeps it for everyone there: the weather
/// its climate's course makes this tick, and how far the clouds have drifted on its winds. A
/// course begun anew starts the clouds over, where the field has them undrifted, so the same
/// course plays the same sky again.
#[derive(Resource, Default)]
pub struct ServerSky {
    began: Option<UnixMillis>,
    along: Option<SpellCourse>,
    weather: Option<Weather>,
    clouds: CloudDrift,
}

impl ServerSky {
    /// The weather over the area this tick, once the sky has turned.
    pub fn weather(&self) -> Option<&Weather> {
        self.weather.as_ref()
    }

    /// How far the clouds have drifted.
    pub fn drift(&self) -> Vec2 {
        self.clouds.drift()
    }
}

/// Turns the sky over the world's area on to this tick: the weather where the server's course
/// stands, the rain as hard as it falls under the cloud that holds the most water, the clouds
/// carried along on its wind, and the hour of the game's day. The world is told that sky, and
/// the scripts of a game that names its times of day hear the day turn from one into the next.
pub fn turn_sky(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let climate = &super::load(&content, world.resource::<WorldArea>().0).climate;
    let look = content.built::<GameSettings>().look.clouds;
    let now = world.resource::<WallClock>().now;
    let seconds = time::uptime(world).0;
    let course = world
        .get_resource::<WeatherClock>()
        .and_then(|clock| clock.0);
    let began = match course {
        Some(TimeCourse::Since(began)) => Some(began),
        Some(TimeCourse::Held(_)) | None => None,
    };
    let mut sky = world.resource_mut::<ServerSky>();
    if sky.began != began {
        *sky = ServerSky { began, ..default() };
    }
    let weather = match course {
        None => climate.first(),
        Some(TimeCourse::Held(spell)) => climate.held(spell),
        Some(course @ TimeCourse::Since(_)) => {
            sky.along = climate.course_at(course, now, sky.along);
            climate.weather_at(sky.along, now)
        }
    };
    sky.clouds.blown(weather.wind.blowing(), seconds, &look);
    sky.weather = Some(weather);
    let cycle = content.built::<GameSettings>().daylight.as_ref();
    let hour = cycle
        .zip(world.get_resource::<DayClock>().and_then(|clock| clock.0))
        .map(|(cycle, course)| cycle.hour(course, now));
    sky::turn(
        world,
        Sky {
            hour,
            weather,
            rain: weather.rain,
        },
    );
    if cycle.is_some_and(|cycle| !cycle.times.is_empty()) {
        sky::raise_turns(world);
    }
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
    /// A light breeze from the south-west under a clear sky, as a day outside has.
    pub fn breezy() -> Weather {
        Weather {
            wind: Wind {
                direction: Degrees(240.0),
                strength: 0.35,
            },
            ..Weather::default()
        }
    }

    /// The weather `share` of the way from this to `other`, the wind turning the shorter way.
    pub fn toward(&self, other: &Weather, share: f32) -> Weather {
        let lerp = |a: f32, b: f32| a + (b - a) * share;
        let blown = |wind: &Wind| {
            let (sin, cos) = wind.direction.radians().0.sin_cos();
            (sin * wind.strength, cos * wind.strength)
        };
        let ((ax, ay), (bx, by)) = (blown(&self.wind), blown(&other.wind));
        let (x, y) = (lerp(ax, bx), lerp(ay, by));
        let direction = match (x, y) {
            (0.0, 0.0) => lerp(self.wind.direction.0, other.wind.direction.0),
            _ => x.atan2(y).to_degrees().rem_euclid(360.0),
        };
        Weather {
            wind: Wind {
                direction: Degrees(direction),
                strength: lerp(self.wind.strength, other.wind.strength),
            },
            clouds: lerp(self.clouds, other.clouds),
            water: lerp(self.water, other.water),
            rain: lerp(self.rain, other.rain),
            lightning: lerp(self.lightning, other.lightning),
        }
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
        cx.range(
            format!("{pointer}/rain"),
            self.rain,
            0.0,
            1.0,
            "of a cloudburst",
        );
        cx.range(
            format!("{pointer}/lightning"),
            self.lightning,
            0.0,
            60.0,
            "strikes a minute",
        );
    }
}

impl Default for Climate {
    fn default() -> Climate {
        Climate::always(Weather::default())
    }
}

impl Climate {
    /// A climate of one spell that never changes: `weather`, for good.
    pub fn always(weather: Weather) -> Climate {
        Climate {
            spells: vec![Spell {
                name: "Always".to_owned(),
                weather,
                lasts: SpellLength {
                    shortest: Seconds(3600.0),
                    longest: Seconds(3600.0),
                },
                next: Vec::new(),
            }],
            turns: Seconds(60.0),
            seed: 0,
            rain: None,
            lightning: None,
        }
    }

    /// Whether the climate is still air under a clear sky for good, as indoors.
    pub fn is_still(&self) -> bool {
        *self == Climate::always(Weather::default())
    }

    /// The spell the course starts at.
    pub fn first(&self) -> Weather {
        self.held(0)
    }

    /// The weather of the spell at `spell` among the climate's spells, held there for good; the
    /// last spell's past the end.
    pub fn held(&self, spell: usize) -> Weather {
        self.spells
            .get(spell)
            .or(self.spells.last())
            .map_or_else(Weather::default, |spell| spell.weather)
    }

    /// The place among the climate's spells of the one named `name`.
    pub fn spell_named(&self, name: &str) -> Option<usize> {
        self.spells.iter().position(|spell| spell.name == name)
    }

    /// The weather `phase` of the way through the spell at `spell`, 0 to 1: the spell's own,
    /// turning toward the one that most likely follows it over the last `turns` of its longest
    /// run, as it does on a turning course.
    pub fn held_at(&self, spell: usize, phase: f32) -> Weather {
        let Some(of) = self.spells.get(spell) else {
            return self.held(spell);
        };
        let next = of
            .next
            .iter()
            .max_by(|a, b| a.weight.total_cmp(&b.weight))
            .and_then(|next| self.spell_named(&next.spell));
        let Some(next) = next else {
            return of.weather;
        };
        let left = of.lasts.longest.0 * (1.0 - phase.clamp(0.0, 1.0));
        let turning = self.turns.0.min(of.lasts.longest.0).max(f32::EPSILON);
        let share = (1.0 - left / turning).clamp(0.0, 1.0);
        of.weather.toward(
            &self.spells[next].weather,
            share * share * (3.0 - 2.0 * share),
        )
    }

    /// Where the course running `course` stands at `now`, from `from` when it last stood there,
    /// so a caller asking again and again walks on from where it last stood.
    pub fn course_at(
        &self,
        course: TimeCourse<usize>,
        now: UnixMillis,
        from: Option<SpellCourse>,
    ) -> Option<SpellCourse> {
        let TimeCourse::Since(origin) = course else {
            return None;
        };
        let mut at = from
            .filter(|at| at.began <= now && at.began >= origin)
            .unwrap_or_else(|| self.step(0, 0, origin));
        while at.ends <= now && !self.spells[at.spell].next.is_empty() {
            at = self.step(at.step + 1, at.next, at.ends);
        }
        Some(at)
    }

    /// The weather where the course stands at `now`: its spell's, turning toward the next over the
    /// last `turns` of the spell; on a held course, the first spell's.
    pub fn weather_at(&self, at: Option<SpellCourse>, now: UnixMillis) -> Weather {
        let Some(at) = at else {
            return self.first();
        };
        let weather = self.spells[at.spell].weather;
        if self.spells[at.spell].next.is_empty() {
            return weather;
        }
        let left = at.ends.since(now).0.max(0.0);
        let turning = self
            .turns
            .0
            .min(at.ends.since(at.began).0)
            .max(f32::EPSILON);
        let share = (1.0 - left / turning).clamp(0.0, 1.0);
        weather.toward(
            &self.spells[at.next].weather,
            share * share * (3.0 - 2.0 * share),
        )
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/spells"),
            self.spells.len(),
            1,
            64,
            "spells",
        );
        cx.at_least(format!("{pointer}/turns"), self.turns.0, 0.0, "seconds");
        for (index, spell) in self.spells.iter().enumerate() {
            let at = format!("{pointer}/spells/{index}");
            cx.text(format!("{at}/name"), &spell.name, 1, 60);
            spell.weather.check(&format!("{at}/weather"), cx);
            cx.range(
                format!("{at}/lasts/shortest"),
                spell.lasts.shortest.0,
                1.0,
                spell.lasts.longest.0,
                "seconds, at most the longest",
            );
            for (follows, next) in spell.next.iter().enumerate() {
                if !self.spells.iter().any(|other| other.name == next.spell) {
                    cx.error(
                        format!("{at}/next/{follows}/spell"),
                        "unknown spell",
                        &next.spell,
                        "the name of one of the climate's spells",
                    );
                }
                cx.at_least(
                    format!("{at}/next/{follows}/weight"),
                    next.weight,
                    0.0,
                    "weight",
                );
            }
            if !spell.next.is_empty() && spell.next.iter().all(|next| next.weight <= 0.0) {
                cx.error(
                    format!("{at}/next"),
                    "no spell follows",
                    "every weight 0",
                    "a weight over 0 for some spell to follow",
                );
            }
        }
        if let Some(rain) = &self.rain {
            rain.check(&format!("{pointer}/rain"), cx);
        }
        if let Some(lightning) = &self.lightning {
            cx.range(
                format!("{pointer}/lightning/flash"),
                lightning.flash,
                0.0,
                16.0,
                "times the open sky's light by day",
            );
            let thunder = lightning.thunder;
            cx.range(
                format!("{pointer}/lightning/thunder/near"),
                thunder.near.0,
                1.0,
                f32::MAX,
                "metres",
            );
            if thunder.reach <= thunder.near {
                cx.error(
                    format!("{pointer}/lightning/thunder/reach"),
                    "out of range",
                    format!("{} m", thunder.reach.0),
                    format!("further than its near of {} m", thunder.near.0),
                );
            }
        }
    }

    /// The `step`th spell of the course, `spell`, beginning at `began`: how long it lasts and
    /// which spell follows it, drawn from the course's seed and the step alone.
    fn step(&self, step: u64, spell: usize, began: UnixMillis) -> SpellCourse {
        let mut rng = Rng::new(self.seed ^ step.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let of = &self.spells[spell.min(self.spells.len() - 1)];
        let lasts =
            of.lasts.shortest.0 + (of.lasts.longest.0 - of.lasts.shortest.0) * rng.rand_float();
        let total: f32 = of.next.iter().map(|next| next.weight.max(0.0)).sum();
        let mut drawn = rng.rand_float() * total;
        let next = of
            .next
            .iter()
            .find(|next| {
                drawn -= next.weight.max(0.0);
                drawn < 0.0 && next.weight > 0.0
            })
            .or_else(|| of.next.iter().rev().find(|next| next.weight > 0.0))
            .and_then(|next| {
                self.spells
                    .iter()
                    .position(|other| other.name == next.spell)
            })
            .unwrap_or(spell);
        SpellCourse {
            step,
            spell,
            next,
            began,
            ends: began.after(Seconds(lasts)),
        }
    }
}

impl Rainfall {
    /// How far the rain is carried sideways for every metre it falls in `wind`, a direction
    /// scaled by the wind's strength (x east, y south).
    pub fn lean(&self, wind: Vec2) -> Vec2 {
        wind * self.slant
    }

    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/slant"),
            self.slant,
            0.0,
            4.0,
            "metres downwind for every metre fallen",
        );
        cx.range(
            format!("{pointer}/shells"),
            self.shells.len(),
            1,
            MAX_RAIN_SHELLS,
            "shells",
        );
        for (index, shell) in self.shells.iter().enumerate() {
            cx.range(
                format!("{pointer}/shells/{index}"),
                shell.0,
                NEAREST_SHELL.0,
                200.0,
                "metres from the eye",
            );
        }
        cx.at_least(format!("{pointer}/wets"), self.wets.0, 0.0, "seconds");
        cx.at_least(format!("{pointer}/dries"), self.dries.0, 0.0, "seconds");
        for (index, sound) in self.sounds.iter().enumerate() {
            let at = format!("{pointer}/sounds/{index}");
            cx.range(
                format!("{at}/rain"),
                sound.rain,
                0.0,
                1.0,
                "of a cloudburst",
            );
            cx.range(format!("{at}/volume"), sound.volume, 0.0, 4.0, "times");
        }
    }
}

impl SpellCourse {
    /// Which of the climate's spells is under way, by its place among them.
    pub fn spell(&self) -> usize {
        self.spell
    }
}

impl Wind {
    /// The way the wind blows over the ground (x east, y south), as long as it is strong.
    pub fn blowing(&self) -> Vec2 {
        let (sin, cos) = self.direction.radians().0.sin_cos();
        Vec2::new(sin, -cos) * self.strength
    }

    pub fn is_calm(&self) -> bool {
        self.strength <= 0.0
    }
}
