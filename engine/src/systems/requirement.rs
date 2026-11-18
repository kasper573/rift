use bevy_ecs::prelude::*;

use crate::core::content::{Content, ContentNames};
use crate::core::script::{Expr, QueryDef, ScriptQuery, Vocabulary, condition};
use crate::core::text::{LineText, SpanText};
use crate::systems::GameSettings;
use crate::systems::area::TimeOfDayName;
use crate::systems::class::ClassId;
use crate::systems::item::inline::InlineItem;
use crate::systems::memory::MemoryId;
use crate::systems::mention::Mention;
use crate::systems::mention::{NameMark, NamedSpan};
use crate::systems::messages::{self, Messages, messages};
use crate::systems::status::StatusEffectId;

/// Where a description of what a script asks finds how what it asks about is named.
pub trait ConditionNames {
    /// How a thing the interface mentions is named: by its name alone, where no more of it is known.
    fn mentioned(&self, mention: Mention) -> LineText;

    /// How a time of day is named.
    fn time_of_day(&self, time: &TimeOfDayName) -> LineText;

    fn memory(&self, memory: MemoryId) -> String;

    fn class(&self, class: ClassId) -> String;

    fn status_effect(&self, effect: StatusEffectId) -> String;
}

impl ConditionNames for Content {
    fn mentioned(&self, mention: Mention) -> LineText {
        match mention {
            Mention::Item(item) => SpanText::from(InlineItem::of(item)).into(),
            other => other.shown(self).span().into(),
        }
    }

    fn time_of_day(&self, time: &TimeOfDayName) -> LineText {
        self.built::<GameSettings>()
            .daylight
            .as_ref()
            .and_then(|cycle| cycle.times.iter().find(|known| known.name == *time))
            .map_or_else(
                || LineText::plain(time.0.clone()),
                |known| {
                    SpanText::from(NamedSpan {
                        name: time.0.clone(),
                        mark: NameMark {
                            icon: Some(known.icon.clone()),
                            color: known.color,
                        },
                    })
                    .into()
                },
            )
    }

    fn memory(&self, memory: MemoryId) -> String {
        self.name_of(memory)
    }

    fn class(&self, class: ClassId) -> String {
        self.name_of(class)
    }

    fn status_effect(&self, effect: StatusEffectId) -> String {
        self.name_of(effect)
    }
}

/// What a query's description reads: how things are named and what the game's messages say.
pub struct Describing<'a> {
    pub names: &'a dyn ConditionNames,
    pub messages: &'a Messages,
}

impl Describing<'_> {
    pub fn words(&self) -> &ConditionMessages {
        &self.messages.condition
    }
}

type Describer = Box<dyn Fn(&ScriptQuery, &Describing) -> Option<LineText> + Send + Sync>;
type Comparer = Box<dyn Fn(&ScriptQuery, &Expr, &Describing) -> Option<LineText> + Send + Sync>;

/// How each kind of query describes itself where a player reads what a condition asks for. Each
/// feature registers the descriptions of its own queries.
#[derive(Default)]
pub struct Describers {
    alone: Vec<Describer>,
    compared: Vec<Comparer>,
}

impl Describers {
    /// What the query `Q` asks about, named.
    pub fn alone<Q: QueryDef>(&mut self, how: fn(&Q, &Describing) -> LineText) -> &mut Describers {
        self.alone.push(Box::new(move |query, describing| {
            query
                .downcast_ref::<Q>()
                .map(|query| how(query, describing))
        }));
        self
    }

    /// What the query `Q` asks, when compared with a number the condition names. It declines
    /// the number it has no words for.
    pub fn compared<Q: QueryDef>(
        &mut self,
        how: fn(&Q, &Expr, &Describing) -> Option<LineText>,
    ) -> &mut Describers {
        self.compared
            .push(Box::new(move |query, limit, describing| {
                how(query.downcast_ref::<Q>()?, limit, describing)
            }));
        self
    }
}

pub fn first_unmet<'a>(
    world: &World,
    subject: Entity,
    conditions: impl IntoIterator<Item = &'a Expr>,
) -> Option<LineText> {
    conditions
        .into_iter()
        .flat_map(condition::parts)
        .find(|part| !condition::holds(world, subject, part))
        .map(|part| {
            let content = world.resource::<Content>();
            describe(content, messages::of(content), part)
        })
}

/// What `condition` asks for, as a player reads it: the things of the game as mentions.
pub fn describe(names: &dyn ConditionNames, messages: &Messages, condition: &Expr) -> LineText {
    let describing = Describing { names, messages };
    let words = describing.words();
    let again = |condition: &Expr| describe(names, messages, condition);
    match condition {
        Expr::Not { of } => match &**of {
            Expr::Query(query) if query.name() == "in_combat" => {
                LineText::plain(words.out_of_combat())
            }
            of => words.not(again(of)),
        },
        Expr::All { of: parts } => parts
            .iter()
            .map(again)
            .reduce(|all, part| words.all_of(all, part))
            .unwrap_or_default(),
        Expr::Any { of: parts } => parts
            .iter()
            .map(again)
            .reduce(|any, part| words.any_of(any, part))
            .unwrap_or_default(),
        Expr::AtLeast { a, b } | Expr::More { a, b } => {
            compared(&describing, a, b).unwrap_or_else(|| words.at_least(again(a), again(b)))
        }
        Expr::Query(query) => describe_query(&describing, query),
        Expr::Bool(true) => LineText::plain(words.truth()),
        Expr::Bool(false) => LineText::plain(words.falsehood()),
        Expr::Int(value) => words.number(*value),
        Expr::Num(value) => words.number(*value),
        Expr::Text(value) => LineText::plain(value.as_str()),
        Expr::Equal { a, b } => words.equal(again(a), again(b)),
        Expr::NotEqual { a, b } => words.not_equal(again(a), again(b)),
        Expr::Less { a, b } => words.less_than(again(a), again(b)),
        Expr::AtMost { a, b } => words.at_most(again(a), again(b)),
        other => LineText::plain(other.kind().name()),
    }
}

fn compared(describing: &Describing, a: &Expr, b: &Expr) -> Option<LineText> {
    let Expr::Query(query) = a else {
        return None;
    };
    let describers = Vocabulary::get().extension_of::<Describers>()?;
    describers
        .compared
        .iter()
        .find_map(|compared| compared(query, b, describing))
}

fn describe_query(describing: &Describing, query: &ScriptQuery) -> LineText {
    Vocabulary::get()
        .extension_of::<Describers>()
        .and_then(|describers| {
            describers
                .alone
                .iter()
                .find_map(|alone| alone(query, describing))
        })
        .unwrap_or_else(|| LineText::plain(query.name()))
}

messages! {
    /// How the game words what a condition asks for, where a player reads it.
    pub struct ConditionMessages {
        /// Being out of combat.
        out_of_combat,
        /// That `{condition}` does not hold.
        not[condition: Line],
        /// Two conditions that must both hold, `{first}` and `{second}`.
        all_of[first: Line, second: Line],
        /// Two conditions of which either must hold, `{first}` and `{second}`.
        any_of[first: Line, second: Line],
        /// That `{a}` is the same as `{b}`.
        equal[a: Line, b: Line],
        /// That `{a}` differs from `{b}`.
        not_equal[a: Line, b: Line],
        /// That `{a}` is at least `{b}`.
        at_least[a: Line, b: Line],
        /// That `{a}` is less than `{b}`.
        less_than[a: Line, b: Line],
        /// That `{a}` is at most `{b}`.
        at_most[a: Line, b: Line],
        /// A condition that always holds.
        truth,
        /// A condition that never holds.
        falsehood,
        /// A plain number, `{value}`, as a condition reads it.
        number[value: Number],
        /// Having at least `{count}` of the item `{item}`.
        items_counted[count: Number, item: Line],
        /// Being at least level `{level}`.
        level_of[level: Number],
        /// Having at least `{amount}` of the stat `{stat}`.
        stat_of[amount: Number, stat: Text],
        /// A memory `{memory}` counted at least `{count}`.
        memory_of[count: Number, memory: Text],
        /// Having at least `{count}` free bag slots.
        free_slots_of[count: Number],
        /// The rank of the skill `{skill}`.
        skill_rank[skill: Line],
        /// The stat `{stat}`, one of `power`, `cooldown`, `range`, `radius`, `duration` or `mana`, of the skill `{skill}`.
        skill_stat[skill: Line, stat: Text],
        /// The item `{item}` offered.
        offered[item: Line],
        /// The item `{item}` worn.
        worn[item: Line],
        /// Being on the quest `{quest}`.
        on_quest[quest: Line],
        /// Having done the quest `{quest}` too lately to do it again.
        quest_cooling[quest: Line],
        /// Having done the task `{task}` of the quest `{quest}`.
        task_done[quest: Line, task: Text],
        /// How long until the quest `{quest}` can be done again.
        quest_resets[quest: Line],
        /// Being in the area `{area}`.
        in_area[area: Line],
        /// Knowing the area `{area}`.
        knows_area[area: Line],
        /// The item `{item}` lying on the ground.
        on_ground[item: Line],
        /// How much of the time of day `{time}` has passed.
        share_of[time: Line],
        /// The player's level.
        level,
        /// Being of the class `{class}`.
        is_class[class: Text],
        /// The skill points the player has spent.
        skill_points_spent,
        /// The power of the status that asks.
        status_power,
        /// Mastering the recipe `{recipe}`.
        mastery[recipe: Text],
        /// Being in a party.
        in_party,
        /// Riding a mount.
        riding,
        /// Being in combat.
        in_combat,
        /// Being in an instanced area.
        in_instance,
        /// The free slots of the bag.
        free_slots,
        /// The items offered.
        items_offered,
        /// The memory `{memory}` set lately.
        set_lately[memory: Text],
        /// Standing on the object `{object}`.
        standing_on[object: Text],
        /// The nearest of something.
        nearest,
        /// How many of something there are.
        count,
        /// Something being there.
        present,
        /// How far something is.
        distance,
        /// How much health something has.
        health,
        /// Being an NPC.
        an_npc,
        /// The one the condition is asked about.
        you,
        /// The thing that asks.
        itself,
        /// What is targeted.
        target,
        /// Walking.
        walking,
        /// Being hostile.
        hostile,
        /// Whom a skill reaches.
        skill_targets,
        /// Where a skill reaches.
        skill_area,
        /// Where the player aims.
        aimed_point,
        /// How hard it rains.
        rain,
        /// The hour of the day.
        hour,
        /// How hard the wind blows.
        wind,
        /// How cloudy the sky is.
        clouds,
        /// Having the skill `{skill}` at rank `{rank}` or above.
        skill_rank_of[rank: Number, skill: Line],
        /// Everyone within some distance of a point.
        around,
    }
}
