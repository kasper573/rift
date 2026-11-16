use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Condition, RiftQuery, RiftScripting, sky};
use crate::core::content::{Content, ContentNames};
use crate::core::script::{Compiler, Eval, Expr, FlowCx, QueryDef, ScriptFault, Value, ValueType};
use crate::systems::area::{AreaId, TimeOfDayName};
use crate::systems::item::ItemId;
use crate::systems::mention::Mention;
use crate::systems::quest::QuestId;
use crate::systems::skill::SkillId;
use crate::systems::text::{self, LineText, NameMark, Span, SpanText};

#[derive(Clone)]
pub struct CompiledCondition {
    eval: Arc<Eval>,
    /// Whether it holds alike whoever it is asked about.
    shared: bool,
}

/// What a condition asks for, as a player reads it: words, with the times of day among them kept
/// as times and the things of the game as mentions, so each can be shown as the game shows it
/// anywhere. Written out it is plain words, a time of day and a thing by their names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Wording(pub Vec<WordingPart>);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WordingPart {
    Words { words: String },
    Time { time: TimeOfDayName },
    Named { name: String, mark: NameMark },
}

/// Where a condition's wording finds how what it asks about is named.
pub trait ConditionNames: ContentNames {
    /// How a thing the interface mentions is named in a wording: by its name alone, where no
    /// more of it is known.
    fn mentioned(&self, mention: Mention) -> Wording {
        Wording::words(match mention {
            Mention::Item(item) => self.name_of(item),
            Mention::Quest(quest) => self.name_of(quest),
            Mention::Skill(skill) => self.name_of(skill),
            Mention::Area(area) => self.name_of(area),
        })
    }
}

impl ConditionNames for Content {
    fn mentioned(&self, mention: Mention) -> Wording {
        let shown = mention.shown(self);
        Wording(vec![WordingPart::Named {
            name: shown.name,
            mark: NameMark {
                icon: Some(shown.icon.path()),
                color: shown.color,
            },
        }])
    }
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
            WordingPart::Named { name, .. } => f.write_str(name),
        })
    }
}

impl From<&Wording> for LineText {
    fn from(wording: &Wording) -> LineText {
        LineText(
            wording
                .0
                .iter()
                .map(|part| match part {
                    WordingPart::Words { words } => SpanText::Text {
                        text: words.clone(),
                        fx: Vec::new(),
                    },
                    WordingPart::Time { time } => SpanText::Text {
                        text: time.0.clone(),
                        fx: Vec::new(),
                    },
                    WordingPart::Named { name, mark } => SpanText::Named {
                        name: name.clone(),
                        mark: mark.clone(),
                    },
                })
                .collect(),
        )
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

pub fn describe(content: &impl ConditionNames, condition: &Condition) -> Wording {
    let between = |a: &Condition, sign: &str, b: &Condition| {
        describe(content, a).then(sign).then(describe(content, b))
    };
    match condition {
        Expr::Not { of } => match &**of {
            Expr::Query(RiftQuery::InCombat { .. }) => "you out of combat".into(),
            of => Wording::words("not ").then(describe(content, of)),
        },
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
                Wording::words(format!("{count} ")).then(content.mentioned(Mention::Item(*item)))
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

fn describe_query(content: &impl ConditionNames, query: &RiftQuery) -> Wording {
    let item = |item: &ItemId| content.mentioned(Mention::Item(*item));
    let skill = |skill: &SkillId| content.mentioned(Mention::Skill(*skill));
    let quest = |quest: &QuestId| content.mentioned(Mention::Quest(*quest));
    let area = |area: &AreaId| content.mentioned(Mention::Area(*area));
    let words = match query {
        RiftQuery::During { time } => return Wording::time(time.clone()),
        RiftQuery::SkillRank { skill: asked, .. } => return skill(asked).then(" rank"),
        RiftQuery::SkillStat {
            skill: asked, stat, ..
        } => return skill(asked).then(format!(" {}", stat.label())),
        RiftQuery::Holding { item: asked, .. } => return item(asked),
        RiftQuery::InstanceHolds { area: asked, .. } => {
            return Wording::words("inside their ").then(area(asked));
        }
        RiftQuery::Offered {
            item: Some(asked), ..
        } => return item(asked).then(" offered"),
        RiftQuery::Equipped { item: asked, .. } => return item(asked).then(" worn"),
        RiftQuery::OnQuest { quest: asked, .. } => return Wording::words("On ").then(quest(asked)),
        RiftQuery::QuestDone { quest: asked, .. } => return quest(asked),
        RiftQuery::QuestCooling { quest: asked, .. } => {
            return quest(asked).then(" done for today");
        }
        RiftQuery::TaskDone {
            quest: asked, task, ..
        } => return quest(asked).then(format!(" · {task}")),
        RiftQuery::QuestResetsIn { quest: asked, .. } => return quest(asked).then(" resets"),
        RiftQuery::InArea { area: asked, .. } => return Wording::words("In ").then(area(asked)),
        RiftQuery::KnowsArea { area: asked, .. } => {
            return Wording::words("Knows ").then(area(asked));
        }
        RiftQuery::GroundItems { item: asked, .. } => return item(asked).then(" on the ground"),
        RiftQuery::TimeShare { time } => {
            return Wording::words("share of ").then(Wording::time(time.clone()));
        }
        RiftQuery::Level { .. } => "Level".to_owned(),
        RiftQuery::IsClass { class, .. } => format!("a {}", content.name_of(*class)),
        RiftQuery::SpentSkillPoints { .. } => "skill points spent".to_owned(),
        RiftQuery::Stat { stat, .. } => stat.label().to_owned(),
        RiftQuery::HasStatus { effect, .. } => content.name_of(*effect),
        RiftQuery::StatusCount { category, .. } => match category {
            Some(category) => format!("{}s", category.label().to_lowercase()),
            None => "statuses".to_owned(),
        },
        RiftQuery::StatusPower => "status power".to_owned(),
        RiftQuery::MastersRecipe { recipe, .. } => format!("{} mastery", recipe.recipe.0),
        RiftQuery::InParty { .. } => "in a party".to_owned(),
        RiftQuery::Riding { .. } => "riding".to_owned(),
        RiftQuery::InCombat { .. } => "in combat".to_owned(),
        RiftQuery::InInstance { .. } => "in an instance".to_owned(),
        RiftQuery::FreeSlots { .. } => "free slots".to_owned(),
        RiftQuery::Offered { .. } => "items offered".to_owned(),
        RiftQuery::Memory { memory, .. } => content.name_of(*memory),
        RiftQuery::SetWithin { memory, .. } => format!("{} lately", content.name_of(*memory)),
        RiftQuery::Inside { object, .. } => format!("Standing on {object}"),
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
