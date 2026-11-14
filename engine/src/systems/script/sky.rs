//! The sky as a script reads it: the hour of the game's day, its times of day and the weather
//! over the area. Each side of the game tells its world what its sky is as a tick or a frame
//! begins, and every condition of the sky alone is then answered once for all that follow it.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Condition, EventKind, RiftEvent, RiftInstances, RiftQueryKind, RiftScripting};
use crate::core::content::{Content, SchemaMark, nearest};
use crate::core::script::{Compiler, Eval, Expr, FlowCx, QueryDef, ScriptFault, Value, ValueType};
use crate::core::time::{DayClock, TimeCourse};
use crate::core::units::HourOfDay;
use crate::systems::GameSettings;
use crate::systems::area::{DayCycle, TimeOfDay, TimeOfDayName, Weather};

/// A condition of the sky alone: the hour, the times of day and the weather, which a server and
/// every one of its clients answer alike. A boolean counts as 0 or 1; a number is held to 0 to 1.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(from = "Condition", into = "Condition")]
#[schemars(transform = SchemaMark::SkyCondition.with(true))]
pub struct SkyCondition {
    condition: Condition,
    /// The condition as it is written, which tells it from every other.
    text: Arc<str>,
}

impl SkyCondition {
    pub fn condition(&self) -> &Condition {
        &self.condition
    }

    /// What keeps the condition from being one every world answers alike, or from being answered
    /// at all.
    pub fn check(&self, content: &Content) -> Result<(), ScriptFault> {
        self.answering(content).map(|_| ())
    }

    /// How much of what follows the condition is there in `world`, 0 to 1, asked afresh: what a
    /// tool asks of a condition no content holds yet.
    pub fn share_in(&self, world: &World) -> f32 {
        self.answering(world.resource::<Content>())
            .map_or(0.0, |answer| share_of(&answer, world))
    }

    /// How much of what follows the condition is there under each of `skies` in turn, in a game
    /// of `content`: what a tool plots over a day.
    pub fn through(
        &self,
        content: &Content,
        skies: impl IntoIterator<Item = Sky>,
    ) -> Result<Vec<f32>, ScriptFault> {
        let answer = self.answering(content)?;
        let mut world = World::new();
        world.insert_resource(content.clone());
        Ok(skies
            .into_iter()
            .map(|sky| {
                world.insert_resource(sky);
                share_of(&answer, &world)
            })
            .collect())
    }

    fn answering(&self, content: &Content) -> Result<Eval, ScriptFault> {
        if let Some(reads) = unshared_read(&self.condition) {
            return Err(ScriptFault::new(
                "reads a player",
                format!(
                    "a condition of the sky alone (the hour, a time of day, the weather), \
                     not {reads}"
                ),
            ));
        }
        let as_a =
            |want| Compiler::<RiftScripting>::standalone(content).expr(&self.condition, want);
        as_a(ValueType::Boolean).or_else(|_| as_a(ValueType::Number))
    }
}

impl From<Condition> for SkyCondition {
    fn from(condition: Condition) -> SkyCondition {
        let text = serde_json::to_string(&condition).expect("a condition is written as json");
        SkyCondition {
            condition,
            text: text.into(),
        }
    }
}

impl From<SkyCondition> for Condition {
    fn from(when: SkyCondition) -> Condition {
        when.condition
    }
}

impl PartialEq for SkyCondition {
    fn eq(&self, other: &SkyCondition) -> bool {
        self.text == other.text
    }
}

impl fmt::Debug for SkyCondition {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The sky over a world's area as its side of the game has it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Sky {
    /// The hour of the game's day; none in a game without one, and before a client is told it.
    pub hour: Option<HourOfDay>,
    pub weather: Weather,
    /// How hard it rains where the sky is asked about with nobody named, 0 none to 1 a
    /// cloudburst.
    pub rain: f32,
}

/// Every distinct sky condition the game's content holds, each ready to be answered: built as the
/// content loads, so however many lights or sounds follow one condition, it is kept once.
#[derive(Default)]
pub struct SkyTable(HashMap<Arc<str>, Eval>);

impl SkyTable {
    /// Keeps `when`, or says what keeps it from being answered.
    pub fn follow(&mut self, content: &Content, when: &SkyCondition) -> Result<(), ScriptFault> {
        if !self.0.contains_key(&when.text) {
            self.0.insert(when.text.clone(), when.answering(content)?);
        }
        Ok(())
    }
}

/// How much every sky condition of the game holds under the world's sky now, each answered once
/// for the sky and the game they were last asked under.
#[derive(Resource, Default)]
pub struct SkyNow {
    shares: HashMap<Arc<str>, f32>,
    answered: Option<(Content, Sky)>,
}

impl SkyNow {
    /// How much every sky condition of a game of `content` holds under `sky`: what a tool asks
    /// of an hour no world is at.
    pub fn under(content: &Content, sky: Sky) -> SkyNow {
        let mut world = World::new();
        world.insert_resource(content.clone());
        world.insert_resource(sky);
        SkyNow::of(&world)
    }

    fn of(world: &World) -> SkyNow {
        let content = world.resource::<Content>();
        SkyNow {
            shares: content
                .try_built::<SkyTable>()
                .into_iter()
                .flat_map(|table| &table.0)
                .map(|(text, answer)| (text.clone(), share_of(answer, world)))
                .collect(),
            answered: Some((content.clone(), now(world))),
        }
    }

    /// Answers the conditions again under the sky `world` was told since, in the game they
    /// were last answered in.
    fn answer_again(&mut self, world: &World) {
        let table = world.resource::<Content>().try_built::<SkyTable>();
        for (text, answer) in table.into_iter().flat_map(|table| &table.0) {
            if let Some(share) = self.shares.get_mut(text) {
                *share = share_of(answer, world);
            }
        }
        if let Some((_, under)) = &mut self.answered {
            *under = now(world);
        }
    }

    /// How much of what follows `when` is there now, 0 to 1: all of it when it follows nothing.
    pub fn share(&self, when: Option<&SkyCondition>) -> f32 {
        when.map_or(1.0, |when| {
            self.shares.get(&when.text).copied().unwrap_or(0.0)
        })
    }
}

/// How much of what follows `when` is there in `world` now.
pub fn share(world: &World, when: Option<&SkyCondition>) -> f32 {
    world
        .get_resource::<SkyNow>()
        .map_or(1.0, |now| now.share(when))
}

/// Tells `world` its sky, and answers every condition followed in it under that sky: nothing
/// when it is the sky its conditions were last answered under, in the same game.
pub fn turn(world: &mut World, sky: Sky) {
    let content = world.resource::<Content>();
    let answered = world
        .get_resource::<SkyNow>()
        .and_then(|now| now.answered.as_ref());
    let same_game = answered.is_some_and(|(game, _)| game.is(content));
    if same_game && answered.is_some_and(|(_, under)| *under == sky) {
        return;
    }
    world.insert_resource(sky);
    if same_game {
        world.resource_scope(|world, mut now: Mut<SkyNow>| now.answer_again(world));
    } else {
        let now = SkyNow::of(world);
        world.insert_resource(now);
    }
}

/// The day's course and the time of day that held as a server's world last looked, to tell a
/// turn of the day from a day set to another hour.
#[derive(Resource, Default)]
pub struct DayWatch(Option<(TimeCourse<HourOfDay>, TimeOfDayName)>);

/// Raises the day's turn from one time of day into the next for the scripts that answer it. A
/// world that starts within a time, or whose day is set to another hour, hears of no turn.
pub fn raise_turns(world: &mut World) {
    let course = world.get_resource::<DayClock>().and_then(|clock| clock.0);
    let holds = course.zip(time_of_day(world).map(|time| &time.name));
    let watched = world.resource::<DayWatch>().0.as_ref();
    if watched.map(|(course, time)| (*course, time)) == holds {
        return;
    }
    let holds = holds.map(|(course, time)| (course, time.clone()));
    let before = std::mem::replace(&mut world.resource_mut::<DayWatch>().0, holds.clone());
    let (Some((course, began)), Some((earlier, ended))) = (holds, before) else {
        return;
    };
    if course != earlier || began == ended {
        return;
    }
    let mut instances = world.resource_mut::<RiftInstances>();
    for (kind, time) in [(EventKind::TimeEnded, ended), (EventKind::TimeBegan, began)] {
        for owner in instances.owners_answering(kind).to_vec() {
            let time = time.clone();
            instances.raise(owner, kind, None, Vec::new(), move |event| match event {
                RiftEvent::TimeBegan { time: named } | RiftEvent::TimeEnded { time: named } => {
                    *named == time
                }
                _ => false,
            });
        }
    }
}

/// The sky `world` was last told.
pub fn now(world: &World) -> Sky {
    world.get_resource::<Sky>().copied().unwrap_or_default()
}

/// The hour of the game's day in `world`; noon in a game without a day.
pub fn hour(world: &World) -> HourOfDay {
    now(world).hour.unwrap_or(HourOfDay::NOON)
}

/// The time of day that holds in `world`, when the game names any.
pub fn time_of_day(world: &World) -> Option<&TimeOfDay> {
    daylight(world)?.time_at(hour(world))
}

pub fn during(world: &World, time: &TimeOfDayName) -> bool {
    time_of_day(world).is_some_and(|holds| holds.name == *time)
}

pub fn time_share(world: &World, time: &TimeOfDayName) -> f32 {
    daylight(world).map_or(0.0, |cycle| cycle.share(time, hour(world)))
}

/// `time` when it is one of the game's times of day, for a query to hold on to.
pub fn known_time(content: &Content, time: &TimeOfDayName) -> Result<TimeOfDayName, ScriptFault> {
    let names: Vec<&String> = content
        .try_built::<GameSettings>()
        .and_then(|settings| settings.daylight.as_ref())
        .into_iter()
        .flat_map(|cycle| &cycle.times)
        .map(|time| &time.name.0)
        .collect();
    if names.contains(&&time.0) {
        return Ok(time.clone());
    }
    let near =
        nearest(names, &time.0).map_or_else(String::new, |near| format!(", nearest {near:?}"));
    Err(ScriptFault::new(
        "unknown time of day",
        format!("one of the game's times of day, not {:?}{near}", time.0),
    ))
}

/// What `condition` reads that is not the same whoever asks, if anything: something of whom it
/// is asked about, or of the script asking.
pub fn unshared_read(condition: &Condition) -> Option<&'static str> {
    let mut unshared = None;
    condition.visit(&mut |expr| {
        let reads = match expr {
            Expr::Query(query) if !query.is_shared() => RiftQueryKind::from(query).into(),
            Expr::Local { .. } | Expr::State { .. } | Expr::Event { .. } => expr.kind().name(),
            _ => return,
        };
        unshared.get_or_insert(reads);
    });
    unshared
}

fn daylight(world: &World) -> Option<&DayCycle> {
    world
        .resource::<Content>()
        .try_built::<GameSettings>()?
        .daylight
        .as_ref()
}

fn share_of(answer: &Eval, world: &World) -> f32 {
    match answer(world, &FlowCx::nobody()) {
        Value::Bool(holds) => f32::from(u8::from(holds)),
        Value::Num(share) => share.clamp(0.0, 1.0) as f32,
        _ => 0.0,
    }
}
