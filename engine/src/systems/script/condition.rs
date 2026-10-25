use std::sync::Arc;

use bevy_ecs::prelude::*;

use super::{Condition, RiftQuery, RiftScripting};
use crate::core::content::{Content, ContentNames};
use crate::core::script::{Compiler, Eval, Expr, FlowCx, ScriptFault, Value, ValueType};
use crate::systems::text::{self, LineText, Span};

#[derive(Clone)]
pub struct CompiledCondition(Arc<Eval>);

impl CompiledCondition {
    pub fn new(content: &Content, condition: &Condition) -> CompiledCondition {
        CompiledCondition(Arc::new(compiled(content, condition, ValueType::Boolean)))
    }

    pub fn holds(&self, world: &World, subject: Entity) -> bool {
        run(world, subject, &self.0).truthy()
    }
}

pub fn holds(world: &World, subject: Entity, condition: &Condition) -> bool {
    eval(world, subject, condition, ValueType::Boolean).truthy()
}

pub fn first_unmet<'a>(
    world: &World,
    subject: Entity,
    conditions: impl IntoIterator<Item = &'a Condition>,
) -> Option<String> {
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

pub fn parts(condition: &Condition) -> Vec<&Condition> {
    match condition {
        Expr::All { of: parts } => parts.iter().collect(),
        other => vec![other],
    }
}

pub fn describe(content: &impl ContentNames, condition: &Condition) -> String {
    match condition {
        Expr::Not { of } => format!("not {}", describe(content, of)),
        Expr::All { of: parts } => parts
            .iter()
            .map(|part| describe(content, part))
            .collect::<Vec<_>>()
            .join(" and "),
        Expr::Any { of: parts } => parts
            .iter()
            .map(|part| describe(content, part))
            .collect::<Vec<_>>()
            .join(" or "),
        Expr::AtLeast { a, b } | Expr::More { a, b } => match (&**a, &**b) {
            (Expr::Query(RiftQuery::Level { .. }), Expr::Int(level)) => format!("Level {level}"),
            (Expr::Query(RiftQuery::Holding { item, .. }), Expr::Int(count)) => {
                format!("{count} {}", content.name_of(*item))
            }
            (Expr::Query(RiftQuery::Stat { stat, .. }), Expr::Num(min)) => {
                format!("{min} {}", stat.label())
            }
            (Expr::Query(RiftQuery::Stat { stat, .. }), Expr::Int(min)) => {
                format!("{min} {}", stat.label())
            }
            (Expr::Query(RiftQuery::Memory { memory, .. }), Expr::Int(count)) => {
                format!("{} {count}", content.name_of(*memory))
            }
            (Expr::Query(RiftQuery::FreeSlots { .. }), Expr::Int(1)) => "1 free slot".to_owned(),
            (Expr::Query(RiftQuery::FreeSlots { .. }), Expr::Int(slots)) => {
                format!("{slots} free slots")
            }
            _ => format!("{} ≥ {}", describe(content, a), describe(content, b)),
        },
        Expr::Query(query) => describe_query(content, query),
        Expr::Bool(value) => value.to_string(),
        Expr::Int(value) => value.to_string(),
        Expr::Num(value) => value.to_string(),
        Expr::Text(value) => (*value).to_owned(),
        Expr::Equal { a, b } => format!("{} is {}", describe(content, a), describe(content, b)),
        Expr::NotEqual { a, b } => {
            format!("{} is not {}", describe(content, a), describe(content, b))
        }
        Expr::Less { a, b } => format!("{} < {}", describe(content, a), describe(content, b)),
        Expr::AtMost { a, b } => format!("{} ≤ {}", describe(content, a), describe(content, b)),
        other => other.kind().name().to_owned(),
    }
}

fn describe_query(content: &impl ContentNames, query: &RiftQuery) -> String {
    match query {
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
        RiftQuery::FreeSlots { .. } => "free slots".to_owned(),
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
        RiftQuery::Target => "target".to_owned(),
        RiftQuery::Walking { .. } => "walking".to_owned(),
        RiftQuery::Hostile { .. } => "hostile".to_owned(),
        RiftQuery::GroundItems { item, .. } => format!("{} on the ground", content.name_of(*item)),
        RiftQuery::SkillTargets => "the skill's targets".to_owned(),
        RiftQuery::SkillArea => "the skill's area".to_owned(),
        RiftQuery::AimedPoint => "the aimed point".to_owned(),
    }
}

fn compiled(content: &Content, condition: &Condition, want: ValueType) -> Eval {
    Compiler::<RiftScripting>::standalone(content)
        .expr(condition, want)
        .unwrap_or_else(|fault| {
            panic!("condition does not compile ({fault}); check_content should have refused it")
        })
}
