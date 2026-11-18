//! The sky as a script reads it: the hour of the game's day, its times of day and the weather
//! over the area. Each side of the game tells its world what its sky is as a tick or a frame
//! begins. A side that shows the sky then answers every condition of the sky alone once for all
//! that follow it; a side that only runs the game answers the few it asks about as it asks.

use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{DayCycle, PLACED_RUNNERS, TimeOfDay, TimeOfDayName, Weather};
use crate::core::content::{Content, ContentProblem, Loading, SchemaMark, nearest};
use crate::core::script::condition::unshared_read;
use crate::core::script::{
    Compiler, Eval, EventDef, EventKind, Expr, FlowCx, Instances, QueryDef, RunnerKind,
    ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::script::{Scripts, owner_site};
use crate::core::text::LineText;
use crate::core::time::{DayClock, TimeCourse};
use crate::core::units::HourOfDay;
use crate::systems::GameSettings;
use crate::systems::movement;
use crate::systems::requirement::Describers;

/// A condition of the sky alone: the hour, the times of day and the weather, which a server and
/// every one of its clients answer alike. A boolean counts as 0 or 1; a number is held to 0 to 1.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(from = "Expr", into = "Expr")]
#[schemars(transform = SchemaMark::SkyCondition.with(true))]
pub struct SkyCondition {
    condition: Expr,
    /// The condition as it is written, which tells it from every other.
    text: Arc<str>,
}

impl SkyCondition {
    pub fn condition(&self) -> &Expr {
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
                "reads a character",
                format!(
                    "a condition of the sky alone (the hour, a time of day, the weather), \
                     not {reads}"
                ),
            ));
        }
        let as_a = |want| Compiler::standalone(content).expr(&self.condition, want);
        as_a(ValueType::Boolean).or_else(|_| as_a(ValueType::Number))
    }
}

impl From<Expr> for SkyCondition {
    fn from(condition: Expr) -> SkyCondition {
        let text = serde_json::to_string(&condition).expect("a condition is written as json");
        SkyCondition {
            condition,
            text: text.into(),
        }
    }
}

impl From<SkyCondition> for Expr {
    fn from(when: SkyCondition) -> Expr {
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

/// How much every sky condition of the game holds under a sky, each answered once for the sky
/// and the game they were last asked under: what reads the sky without its world reads here.
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

    /// How much of what follows `when` is there under the sky last answered, 0 to 1: all of it
    /// when it follows nothing.
    pub fn share(&self, when: Option<&SkyCondition>) -> f32 {
        when.map_or(1.0, |when| {
            self.shares.get(&when.text).copied().unwrap_or(0.0)
        })
    }
}

/// How much of what follows `when` is there in `world` now: as answered for all that follow it
/// where the world's sky has been, else asked of the world.
pub fn share(world: &World, when: Option<&SkyCondition>) -> f32 {
    let (Some(when), Some(content)) = (when, world.get_resource::<Content>()) else {
        return 1.0;
    };
    let answered = world.get_resource::<SkyNow>().filter(|answers| {
        answers
            .answered
            .as_ref()
            .is_some_and(|(game, under)| game.is(content) && *under == now(world))
    });
    match answered {
        Some(answers) => answers.share(Some(when)),
        None => content
            .try_built::<SkyTable>()
            .and_then(|table| table.0.get(&when.text))
            .map_or(0.0, |answer| share_of(answer, world)),
    }
}

/// Tells `world` its sky.
pub fn turn(world: &mut World, sky: Sky) {
    match world.get_resource_mut::<Sky>() {
        Some(mut told) => {
            told.set_if_neq(sky);
        }
        None => world.insert_resource(sky),
    }
}

/// Answers every condition followed in `world` under the sky it was told, for what reads the
/// answers without the world: nothing when that is the sky they were last answered under, in the
/// same game.
pub fn answer(world: &mut World) {
    let content = world.resource::<Content>();
    let answered = world
        .get_resource::<SkyNow>()
        .and_then(|now| now.answered.as_ref());
    let same_game = answered.is_some_and(|(game, _)| game.is(content));
    if same_game && answered.is_some_and(|(_, under)| *under == now(world)) {
        return;
    }
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
    let mut instances = world.resource_mut::<Instances>();
    for (kind, time) in [
        (EventKind::of::<TimeEnded>(), ended),
        (EventKind::of::<TimeBegan>(), began),
    ] {
        for owner in instances.owners_answering(kind).to_vec() {
            let time = time.clone();
            instances.raise(owner, kind, None, Vec::new(), move |event| {
                event
                    .downcast_ref::<TimeBegan>()
                    .map(|turn| &turn.time)
                    .or_else(|| event.downcast_ref::<TimeEnded>().map(|turn| &turn.time))
                    .is_some_and(|named| *named == time)
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

/// The day turned into the time of day. A world that starts within it hears nothing.
#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct TimeBegan {
    pub time: TimeOfDayName,
}

/// The game's day turned out of the time of day.
#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct TimeEnded {
    pub time: TimeOfDayName,
}

impl EventDef for TimeBegan {
    fn owners(&self) -> &'static [RunnerKind] {
        PLACED_RUNNERS
    }

    fn has_subject(&self) -> bool {
        false
    }
}

impl EventDef for TimeEnded {
    fn owners(&self) -> &'static [RunnerKind] {
        PLACED_RUNNERS
    }

    fn has_subject(&self) -> bool {
        false
    }
}

/// How hard rain falls on where someone or something stands, 0 none to 1 a cloudburst; none under
/// what shelters from it.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Rain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

/// The hour of the game's day, 0 to 24; noon in a game without a day.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Hour {}

/// Whether the time of day holds.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct During {
    pub time: TimeOfDayName,
}

/// How much of the time of day has come, 0 outside it to 1 within, easing between.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct TimeShare {
    pub time: TimeOfDayName,
}

/// How strongly the wind blows over the area, from calm at 0 to a gale at 1.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Wind {}

/// How much of the area's sky is clouded over, 0 to 1.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Clouds {}

impl QueryDef for Rain {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        self.of.is_none()
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let named = self.of.is_some();
        let of = cx.actor(self.of.as_deref())?;
        Ok(Box::new(move |world, cx| {
            let rain = if named || cx.subject.is_some() {
                crate::systems::aim::actor(world, cx, &of)
                    .and_then(|who| movement::position(world, who))
                    .map_or(0.0, |at| super::rain::falling_on(world, at))
            } else {
                now(world).rain
            };
            Value::Num(f64::from(rain))
        }))
    }
}

impl QueryDef for Hour {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        true
    }

    fn compile<'c>(&'c self, _: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|world, _| Value::Num(f64::from(hour(world).0))))
    }
}

impl QueryDef for During {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        true
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let time = known_time(cx.content, &self.time)?;
        Ok(Box::new(move |world, _| Value::Bool(during(world, &time))))
    }
}

impl QueryDef for TimeShare {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        true
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let time = known_time(cx.content, &self.time)?;
        Ok(Box::new(move |world, _| {
            Value::Num(f64::from(time_share(world, &time)))
        }))
    }
}

impl QueryDef for Wind {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        true
    }

    fn compile<'c>(&'c self, _: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|world, _| {
            Value::Num(f64::from(now(world).weather.wind.strength))
        }))
    }
}

impl QueryDef for Clouds {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        true
    }

    fn compile<'c>(&'c self, _: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok(Box::new(|world, _| {
            Value::Num(f64::from(now(world).weather.clouds))
        }))
    }
}

/// The times of day `condition` names, each once, in the order it names them.
pub fn times(condition: &Expr) -> Vec<TimeOfDayName> {
    let mut times: Vec<TimeOfDayName> = Vec::new();
    condition.visit(&mut |expr| {
        let Expr::Query(query) = expr else {
            return;
        };
        let named = query
            .downcast_ref::<During>()
            .map(|asked| &asked.time)
            .or_else(|| query.downcast_ref::<TimeShare>().map(|asked| &asked.time));
        if let Some(time) = named
            && !times.contains(time)
        {
            times.push(time.clone());
        }
    });
    times
}

/// Registers what scripts ask and hear of the sky.
pub fn script(vocabulary: &mut Vocabulary) {
    vocabulary
        .event::<TimeBegan>("time_began")
        .event::<TimeEnded>("time_ended")
        .query::<Rain>("rain")
        .query::<Hour>("hour")
        .query::<During>("during")
        .query::<TimeShare>("time_share")
        .query::<Wind>("wind")
        .query::<Clouds>("clouds");
    vocabulary
        .extension::<Describers>()
        .alone::<Rain>(|_, d| LineText::plain(d.words().rain()))
        .alone::<Hour>(|_, d| LineText::plain(d.words().hour()))
        .alone::<During>(|q, d| d.names.time_of_day(&q.time))
        .alone::<TimeShare>(|q, d| d.words().share_of(d.names.time_of_day(&q.time)))
        .alone::<Wind>(|_, d| LineText::plain(d.words().wind()))
        .alone::<Clouds>(|_, d| LineText::plain(d.words().clouds()));
}

/// Every time of day a script's event names must be one the game's day has.
pub fn check_times(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    let turns = scripts
        .hearing::<TimeBegan>()
        .into_iter()
        .map(|heard| (heard.row, heard.label, &heard.event.time))
        .chain(
            scripts
                .hearing::<TimeEnded>()
                .into_iter()
                .map(|heard| (heard.row, heard.label, &heard.event.time)),
        );
    for (row, label, time) in turns {
        if let Err(fault) = known_time(&content, time) {
            let site = owner_site(&content, row);
            loading.json_error(
                &site.file,
                &site.pointer,
                ContentProblem::new(fault.rule, label.to_owned(), fault.expected),
            );
        }
    }
}
