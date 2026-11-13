use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Condition, RiftQuery, RiftScripting, sky};
use crate::core::content::{Content, ContentNames};
use crate::core::script::{Compiler, Eval, Expr, FlowCx, QueryDef, ScriptFault, Value, ValueType};
use crate::systems::area::TimeOfDayName;
use crate::systems::text::{self, LineText, Span};

#[derive(Clone)]
pub struct CompiledCondition {
    eval: Arc<Eval>,
    /// Whether it holds alike whoever it is asked about.
    shared: bool,
}

/// What a condition asks for, as a player reads it: words, with the times of day among them kept
/// as times, so they can be shown as the game shows its times. Written out it is plain words, a
/// time of day by its name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Wording(pub Vec<WordingPart>);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WordingPart {
    Words { words: String },
    Time { time: TimeOfDayName },
}

impl Wording {
    pub fn words(words: impl Into<String>) -> Wording {
        Wording(vec![WordingPart::Words {
            words: words.into(),
        }])
    }

    pub fn time(time: TimeOfDayName) -> Wording {
        Wording(vec![WordingPart::Time { time }])
    }

    /// This wording with `more` after it.
    pub fn then(mut self, more: impl Into<Wording>) -> Wording {
        for part in more.into().0 {
            match (self.0.last_mut(), part) {
                (Some(WordingPart::Words { words }), WordingPart::Words { words: more }) => {
                    words.push_str(&more);
                }
                (_, part) => self.0.push(part),
            }
        }
        self
    }

    fn joined(parts: impl IntoIterator<Item = Wording>, between: &str) -> Wording {
        parts
            .into_iter()
            .reduce(|all, part| all.then(between).then(part))
            .unwrap_or_default()
    }
}

impl From<String> for Wording {
    fn from(words: String) -> Wording {
        Wording::words(words)
    }
}

impl From<&str> for Wording {
    fn from(words: &str) -> Wording {
        Wording::words(words)
    }
}

impl fmt::Display for Wording {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.iter().try_for_each(|part| match part {
            WordingPart::Words { words } => f.write_str(words),
            WordingPart::Time { time } => f.write_str(&time.0),
        })
    }
}

impl CompiledCondition {
    pub fn new(content: &Content, condition: &Condition) -> CompiledCondition {
        let mut reads_someone = sky::unshared_read(condition).is_some();
        condition.visit(&mut |expr| {
            if let Expr::Query(query) = expr {
                reads_someone |= query.reads_subject();
            }
        });
        CompiledCondition {
            eval: Arc::new(compiled(content, condition, ValueType::Boolean)),
            shared: !reads_someone,
        }
    }

    pub fn holds(&self, world: &World, subject: Entity) -> bool {
        run(world, subject, &self.eval).truthy()
    }

    /// Whether it holds for everyone, when it holds alike whoever it is asked about.
    pub fn shared(&self, world: &World) -> Option<bool> {
        self.shared
            .then(|| (self.eval)(world, &FlowCx::nobody()).truthy())
    }
}

pub fn holds(world: &World, subject: Entity, condition: &Condition) -> bool {
    eval(world, subject, condition, ValueType::Boolean).truthy()
}

pub fn first_unmet<'a>(
    world: &World,
    subject: Entity,
    conditions: impl IntoIterator<Item = &'a Condition>,
) -> Option<Wording> {
    conditions
        .into_iter()
        .flat_map(parts)
        .find(|part| !holds(world, subject, part))
        .map(|part| describe(world.resource::<Content>(), part))
}

pub fn spoken(world: &World, subject: Entity, spans: &[Span]) -> LineText {
    let values: Vec<Value> = text::values(spans)
        .into_iter()
        .map(|value| eval(world, subject, value, ValueType::Text))
        .collect();
    LineText::spoken(spans, &values)
}

fn eval(world: &World, subject: Entity, expr: &Condition, want: ValueType) -> Value {
    run(
        world,
        subject,
        &compiled(world.resource::<Content>(), expr, want),
    )
}

fn run(world: &World, subject: Entity, compiled: &Eval) -> Value {
    compiled(world, &FlowCx::standalone(subject))
}

pub fn check(content: &Content, condition: &Condition) -> Result<(), ScriptFault> {
    Compiler::<RiftScripting>::standalone(content)
        .expr(condition, ValueType::Boolean)
        .map(|_| ())
}

pub fn check_value(content: &Content, value: &Condition) -> Result<(), ScriptFault> {
    Compiler::<RiftScripting>::standalone(content)
        .expr(value, ValueType::Text)
        .map(|_| ())
}

pub fn min_level(part: &Condition) -> Option<u32> {
    match part {
        Expr::AtLeast { a, b } => match (&**a, &**b) {
            (Expr::Query(RiftQuery::Level { .. }), Expr::Int(level)) => u32::try_from(*level).ok(),
            _ => None,
        },
        _ => None,
    }
}

pub fn standing(world: &World, subject: Entity, condition: &Condition) -> Option<String> {
    match condition {
        Expr::AtLeast { a, .. } | Expr::More { a, .. } => match &**a {
            Expr::Query(RiftQuery::Level { .. }) => Some(format!(
                "you are level {}",
                crate::systems::class::level(world, subject)
            )),
            _ => None,
        },
        _ => None,
    }
}

/// The times of day `condition` names, each once, in the order it names them.
pub fn times(condition: &Condition) -> Vec<TimeOfDayName> {
    let mut times: Vec<TimeOfDayName> = Vec::new();
    condition.visit(&mut |expr| {
        if let Expr::Query(RiftQuery::During { time } | RiftQuery::TimeShare { time }) = expr
            && !times.contains(time)
        {
            times.push(time.clone());
        }
    });
    times
}

pub fn parts(condition: &Condition) -> Vec<&Condition> {
    match condition {
        Expr::All { of: parts } => parts.iter().collect(),
        other => vec![other],
    }
}

pub fn describe(content: &impl ContentNames, condition: &Condition) -> Wording {
    let between = |a: &Condition, sign: &str, b: &Condition| {
        describe(content, a).then(sign).then(describe(content, b))
    };
    match condition {
        Expr::Not { of } => Wording::words("not ").then(describe(content, of)),
        Expr::All { of: parts } => {
            Wording::joined(parts.iter().map(|part| describe(content, part)), " and ")
        }
        Expr::Any { of: parts } => {
            Wording::joined(parts.iter().map(|part| describe(content, part)), " or ")
        }
        Expr::AtLeast { a, b } | Expr::More { a, b } => match (&**a, &**b) {
            (Expr::Query(RiftQuery::Level { .. }), Expr::Int(level)) => {
                format!("Level {level}").into()
            }
            (Expr::Query(RiftQuery::Holding { item, .. }), Expr::Int(count)) => {
                format!("{count} {}", content.name_of(*item)).into()
            }
            (Expr::Query(RiftQuery::Stat { stat, .. }), Expr::Num(min)) => {
                format!("{min} {}", stat.label()).into()
            }
            (Expr::Query(RiftQuery::Stat { stat, .. }), Expr::Int(min)) => {
                format!("{min} {}", stat.label()).into()
            }
            (Expr::Query(RiftQuery::Memory { memory, .. }), Expr::Int(count)) => {
                format!("{} {count}", content.name_of(*memory)).into()
            }
            (Expr::Query(RiftQuery::FreeSlots { .. }), Expr::Int(1)) => "1 free slot".into(),
            (Expr::Query(RiftQuery::FreeSlots { .. }), Expr::Int(slots)) => {
                format!("{slots} free slots").into()
            }
            _ => between(a, " ≥ ", b),
        },
        Expr::Query(query) => describe_query(content, query),
        Expr::Bool(value) => value.to_string().into(),
        Expr::Int(value) => value.to_string().into(),
        Expr::Num(value) => value.to_string().into(),
        Expr::Text(value) => value.as_str().into(),
        Expr::Equal { a, b } => between(a, " is ", b),
        Expr::NotEqual { a, b } => between(a, " is not ", b),
        Expr::Less { a, b } => between(a, " < ", b),
        Expr::AtMost { a, b } => between(a, " ≤ ", b),
        other => other.kind().name().into(),
    }
}

fn describe_query(content: &impl ContentNames, query: &RiftQuery) -> Wording {
    let words = match query {
        RiftQuery::During { time } => return Wording::time(time.clone()),
        RiftQuery::TimeShare { time } => {
            return Wording::words("share of ").then(Wording::time(time.clone()));
        }
        RiftQuery::Level { .. } => "Level".to_owned(),
        RiftQuery::IsClass { class, .. } => format!("a {}", content.name_of(*class)),
        RiftQuery::SkillRank { skill, .. } => format!("{} rank", content.name_of(*skill)),
        RiftQuery::SkillStat { skill, stat, .. } => {
            format!("{} {}", content.name_of(*skill), stat.label())
        }
        RiftQuery::SpentSkillPoints { .. } => "skill points spent".to_owned(),
        RiftQuery::Stat { stat, .. } => stat.label().to_owned(),
        RiftQuery::HasStatus { effect, .. } => content.name_of(*effect),
        RiftQuery::StatusCount { category, .. } => match category {
            Some(category) => format!("{}s", category.label().to_lowercase()),
            None => "statuses".to_owned(),
        },
        RiftQuery::StatusPower => "status power".to_owned(),
        RiftQuery::Holding { item, .. } => content.name_of(*item),
        RiftQuery::MastersRecipe { recipe, .. } => format!("{} mastery", recipe.recipe.0),
        RiftQuery::FreeSlots { .. } => "free slots".to_owned(),
        RiftQuery::Offered { item, .. } => match item {
            Some(item) => format!("{} offered", content.name_of(*item)),
            None => "items offered".to_owned(),
        },
        RiftQuery::Equipped { item, .. } => format!("{} worn", content.name_of(*item)),
        RiftQuery::Memory { memory, .. } => content.name_of(*memory),
        RiftQuery::SetWithin { memory, .. } => format!("{} lately", content.name_of(*memory)),
        RiftQuery::OnQuest { quest, .. } => format!("On {}", content.name_of(*quest)),
        RiftQuery::QuestDone { quest, .. } => content.name_of(*quest),
        RiftQuery::QuestCooling { quest, .. } => {
            format!("{} done for today", content.name_of(*quest))
        }
        RiftQuery::TaskDone { quest, task, .. } => format!("{} · {task}", content.name_of(*quest)),
        RiftQuery::QuestResetsIn { quest, .. } => format!("{} resets", content.name_of(*quest)),
        RiftQuery::Inside { object, .. } => format!("Standing on {object}"),
        RiftQuery::InArea { area, .. } => format!("In {}", content.name_of(*area)),
        RiftQuery::KnowsArea { area, .. } => format!("Knows {}", content.name_of(*area)),
        RiftQuery::Nearest { .. } => "nearest".to_owned(),
        RiftQuery::Count { .. } => "count".to_owned(),
        RiftQuery::Present { .. } => "present".to_owned(),
        RiftQuery::Distance { .. } => "distance".to_owned(),
        RiftQuery::Health { .. } => "health".to_owned(),
        RiftQuery::IsNpc { .. } => "an npc".to_owned(),
        RiftQuery::Subject => "you".to_owned(),
        RiftQuery::Owner => "itself".to_owned(),
        RiftQuery::Target => "target".to_owned(),
        RiftQuery::Walking { .. } => "walking".to_owned(),
        RiftQuery::Hostile { .. } => "hostile".to_owned(),
        RiftQuery::GroundItems { item, .. } => format!("{} on the ground", content.name_of(*item)),
        RiftQuery::SkillTargets => "the skill's targets".to_owned(),
        RiftQuery::SkillArea => "the skill's area".to_owned(),
        RiftQuery::AimedPoint => "the aimed point".to_owned(),
        RiftQuery::Rain { .. } => "rain".to_owned(),
        RiftQuery::Hour => "the hour".to_owned(),
        RiftQuery::Wind => "wind".to_owned(),
        RiftQuery::Clouds => "clouds".to_owned(),
    };
    words.into()
}

fn compiled(content: &Content, condition: &Condition, want: ValueType) -> Eval {
    Compiler::<RiftScripting>::standalone(content)
        .expr(condition, want)
        .unwrap_or_else(|fault| {
            panic!("condition does not compile ({fault}); check_content should have refused it")
        })
}
