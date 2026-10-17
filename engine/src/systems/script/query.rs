use bevy_ecs::prelude::*;
use bevy_ecs::query::Has;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Condition, RiftScripting};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{Compiler, Eval, FlowCx, QueryDef, ScriptFault, Value, ValueType};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{Seconds, WallClock};
use crate::systems::area::transition::Discovered;
use crate::systems::area::{self, Area, AreaId, AreaTag, object};
use crate::systems::class::{self, ClassId};
use crate::systems::combat::{Attackers, Attitude};
use crate::systems::equipment::Equipment;
use crate::systems::item::{DroppedItem, Inventory, ItemId};
use crate::systems::memory::{self, MemoryId};
use crate::systems::movement::{self, Position};
use crate::systems::npc::{Npc, NpcId, Pack, body};
use crate::systems::player::{ArrivalHold, Owner as PlayerOwner};
use crate::systems::quest::{self, Objective, QuestId};
use crate::systems::script::OwnerKind;
use crate::systems::skill::{self, SkillDef, SkillId, SkillStat};
use crate::systems::stat::{self, StatKind, Stats};
use crate::systems::status::{self, StatusCategory, StatusEffectDef, StatusEffectId};
use crate::systems::visibility::{self, Presence};

/// Who stood where as the tick began. Every script in a tick counts the same actors, whatever order
/// the scripts run in.
#[derive(Resource, Default)]
pub struct TickRoster {
    listed: Vec<Listed>,
    characters: usize,
    character_bounds: Option<Bounds>,
    npc_bounds: Option<Bounds>,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Pos<Tiles>,
    max: Pos<Tiles>,
}

impl Bounds {
    fn around(listed: &[Listed]) -> Option<Bounds> {
        let mut entries = listed.iter();
        let first = entries.next()?.at;
        Some(entries.fold(
            Bounds {
                min: first,
                max: first,
            },
            |bounds, entry| Bounds {
                min: Pos::new(bounds.min.x.min(entry.at.x), bounds.min.y.min(entry.at.y)),
                max: Pos::new(bounds.max.x.max(entry.at.x), bounds.max.y.max(entry.at.y)),
            },
        ))
    }

    fn reaches(self, from: Pos<Tiles>, within: Tiles) -> bool {
        from.x + within.0 >= self.min.x
            && from.x - within.0 <= self.max.x
            && from.y + within.0 >= self.min.y
            && from.y - within.0 <= self.max.y
    }
}

#[derive(Clone, Copy)]
pub struct Listed {
    pub entity: Entity,
    pub at: Pos<Tiles>,
    pub area: AreaId,
    pub npc: Option<NpcId>,
    pub character: bool,
    pub dead: bool,
    pub held: bool,
    pub presence: bool,
}

impl TickRoster {
    pub fn iter(&self) -> impl Iterator<Item = &Listed> {
        self.listed.iter()
    }

    pub fn among(&self, among: Among) -> &[Listed] {
        match among {
            Among::Characters => &self.listed[..self.characters],
            Among::Npcs => &self.listed[self.characters..],
            Among::Anyone | Among::Attackers | Among::PackAttackers => &self.listed,
        }
    }

    fn may_reach(&self, among: Among, from: Pos<Tiles>, within: Tiles) -> bool {
        let reaches =
            |bounds: Option<Bounds>| bounds.is_some_and(|bounds| bounds.reaches(from, within));
        match among {
            Among::Characters => reaches(self.character_bounds),
            Among::Npcs => reaches(self.npc_bounds),
            Among::Anyone | Among::Attackers | Among::PackAttackers => {
                reaches(self.character_bounds) || reaches(self.npc_bounds)
            }
        }
    }
}

type RosterQuery = QueryState<
    (
        Entity,
        &'static Position,
        &'static AreaTag,
        Option<&'static Npc>,
        Option<&'static Stats>,
        Has<PlayerOwner>,
        Has<ArrivalHold>,
        Has<Presence>,
    ),
    Or<(With<PlayerOwner>, With<Npc>)>,
>;

pub fn take_tick_roster(world: &mut World, query: &mut RosterQuery) {
    let mut listed = std::mem::take(&mut world.get_resource_or_init::<TickRoster>().listed);
    listed.clear();
    listed.extend(query.iter(world).map(
        |(entity, at, tag, npc, stats, character, held, presence)| Listed {
            entity,
            at: at.pos,
            area: tag.area,
            npc: npc.map(|npc| npc.def),
            character,
            dead: stats.is_some_and(stat::dead),
            held,
            presence,
        },
    ));
    listed.sort_by_key(|entry| !entry.character);
    let characters = listed.iter().take_while(|entry| entry.character).count();
    let mut roster = world.resource_mut::<TickRoster>();
    roster.character_bounds = Bounds::around(&listed[..characters]);
    roster.npc_bounds = Bounds::around(&listed[characters..]);
    roster.listed = listed;
    roster.characters = characters;
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Among {
    Characters,
    Npcs,
    Attackers,
    PackAttackers,
    Anyone,
}

type Of = Option<Box<Condition>>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RiftQuery {
    Level {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
    },
    IsClass {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        class: ClassId,
    },
    SkillRank {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        skill: SkillId,
    },
    SkillStat {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        skill: SkillId,
        stat: SkillStat,
    },
    SpentSkillPoints {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
    },
    Stat {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        stat: StatKind,
    },
    HasStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        effect: StatusEffectId,
    },
    StatusCount {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<StatusCategory>,
    },
    StatusPower,
    Holding {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        item: ItemId,
    },
    FreeSlots {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
    },
    Wearing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        item: ItemId,
    },
    Memory {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        memory: MemoryId,
    },
    SetWithin {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        memory: MemoryId,
        seconds: Box<Condition>,
    },
    OnQuest {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        quest: QuestId,
    },
    QuestDone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        quest: QuestId,
    },
    QuestCooling {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        quest: QuestId,
    },
    TaskDone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        quest: QuestId,
        task: String,
    },
    QuestResetsIn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        quest: QuestId,
    },
    Inside {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        object: String,
    },
    InArea {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        area: AreaId,
    },
    KnowsArea {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Of,
        area: AreaId,
    },
    Nearest {
        among: Among,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        npc: Option<NpcId>,
        within: Box<Condition>,
    },
    Count {
        among: Among,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        npc: Option<NpcId>,
        within: Box<Condition>,
    },
    Present {
        actor: Box<Condition>,
    },
    Distance {
        actor: Box<Condition>,
    },
    Health {
        actor: Box<Condition>,
    },
    IsNpc {
        actor: Box<Condition>,
    },
    Subject,
    Target,
    Walking {
        actor: Box<Condition>,
    },
    Hostile {
        actor: Box<Condition>,
    },
    GroundItems {
        item: ItemId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Box<Condition>>,
        within: Box<Condition>,
    },
    SkillTargets,
    SkillArea,
    AimedPoint,
}

impl QueryDef<RiftScripting> for RiftQuery {
    fn value_type(&self, cx: &Compiler<RiftScripting>) -> ValueType {
        match self {
            RiftQuery::Level { .. }
            | RiftQuery::SkillRank { .. }
            | RiftQuery::SpentSkillPoints { .. }
            | RiftQuery::Holding { .. }
            | RiftQuery::FreeSlots { .. }
            | RiftQuery::Count { .. }
            | RiftQuery::StatusCount { .. }
            | RiftQuery::GroundItems { .. } => ValueType::Integer,
            RiftQuery::Stat { .. }
            | RiftQuery::SkillStat { .. }
            | RiftQuery::StatusPower
            | RiftQuery::Distance { .. }
            | RiftQuery::Health { .. } => ValueType::Number,
            RiftQuery::Memory { memory, .. } => memory.get(cx.content).ty,
            RiftQuery::QuestResetsIn { .. } => ValueType::Text,
            RiftQuery::Nearest { .. } | RiftQuery::Target | RiftQuery::Subject => ValueType::Actor,
            RiftQuery::SkillTargets => ValueType::Actors,
            RiftQuery::SkillArea => ValueType::Area,
            RiftQuery::AimedPoint => ValueType::Point,
            _ => ValueType::Boolean,
        }
    }

    fn reads_subject(&self) -> bool {
        match self {
            RiftQuery::Level { of }
            | RiftQuery::IsClass { of, .. }
            | RiftQuery::SkillRank { of, .. }
            | RiftQuery::SkillStat { of, .. }
            | RiftQuery::SpentSkillPoints { of }
            | RiftQuery::Stat { of, .. }
            | RiftQuery::HasStatus { of, .. }
            | RiftQuery::StatusCount { of, .. }
            | RiftQuery::Holding { of, .. }
            | RiftQuery::FreeSlots { of }
            | RiftQuery::Wearing { of, .. }
            | RiftQuery::Memory { of, .. }
            | RiftQuery::SetWithin { of, .. }
            | RiftQuery::OnQuest { of, .. }
            | RiftQuery::QuestDone { of, .. }
            | RiftQuery::QuestCooling { of, .. }
            | RiftQuery::TaskDone { of, .. }
            | RiftQuery::QuestResetsIn { of, .. }
            | RiftQuery::Inside { of, .. }
            | RiftQuery::InArea { of, .. }
            | RiftQuery::KnowsArea { of, .. } => of.is_none(),
            RiftQuery::SkillTargets
            | RiftQuery::SkillArea
            | RiftQuery::AimedPoint
            | RiftQuery::StatusPower => true,
            _ => false,
        }
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c, RiftScripting>) -> Result<Eval, ScriptFault> {
        Ok(match self {
            RiftQuery::Level { of } => {
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(
                        character(world, cx, &of)
                            .map_or(0, |who| i64::from(class::level(world, who))),
                    )
                })
            }
            RiftQuery::IsClass { of, class } => {
                let class = *class;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        character(world, cx, &of)
                            .is_some_and(|who| class::of(world, who) == Some(class)),
                    )
                })
            }
            RiftQuery::SkillRank { of, skill } => {
                let skill = *skill;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(
                        actor(world, cx, &of)
                            .map_or(0, |who| i64::from(skill::rank_of(world, who, skill).0)),
                    )
                })
            }
            RiftQuery::SkillStat { of, skill, stat } => {
                let (skill, stat) = (*skill, *stat);
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    let rank = actor(world, cx, &of)
                        .map_or(skill::Rank::NONE, |who| skill::rank_of(world, who, skill));
                    Value::Num(f64::from(
                        skill.get(world.resource::<Content>()).stat(rank, stat),
                    ))
                })
            }
            RiftQuery::SpentSkillPoints { of } => {
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(
                        character(world, cx, &of)
                            .map_or(0, |who| i64::from(class::spent_points(world, who).0)),
                    )
                })
            }
            RiftQuery::Stat { of, stat } => {
                let stat = *stat;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Num(
                        actor(world, cx, &of)
                            .map_or(0.0, |who| f64::from(stat::effective(world, who, stat))),
                    )
                })
            }
            RiftQuery::HasStatus { of, effect } => {
                let effect = *effect;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        actor(world, cx, &of).is_some_and(|who| status::has(world, who, effect)),
                    )
                })
            }
            RiftQuery::StatusCount { of, category } => {
                let category = *category;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(actor(world, cx, &of).map_or(0, |who| {
                        world.get::<status::Statuses>(who).map_or(0, |statuses| {
                            statuses.count(world.resource::<Content>(), category) as i64
                        })
                    }))
                })
            }
            RiftQuery::StatusPower => {
                let effect = status_of(cx)?;
                Box::new(move |world, cx| {
                    Value::Num(
                        cx.subject
                            .map_or(0.0, |who| f64::from(status::power(world, who, effect))),
                    )
                })
            }
            RiftQuery::Holding { of, item } => {
                let item = *item;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(character(world, cx, &of).map_or(0, |who| {
                        world
                            .get::<Inventory>(who)
                            .map_or(0, |bag| i64::from(bag.count(item)))
                    }))
                })
            }
            RiftQuery::FreeSlots { of } => {
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Int(character(world, cx, &of).map_or(0, |who| {
                        world
                            .get::<Inventory>(who)
                            .map_or(0, |bag| i64::from(bag.free_slots()))
                    }))
                })
            }
            RiftQuery::Wearing { of, item } => {
                let item = *item;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        world
                            .get::<Equipment>(who)
                            .is_some_and(|worn| worn.slots.values().any(|worn| *worn == item))
                    }))
                })
            }
            RiftQuery::Memory { of, memory } => {
                let memory = *memory;
                let of = cx.actor(of.as_deref())?;
                let ty = memory.get(cx.content).ty;
                Box::new(move |world, cx| {
                    character(world, cx, &of)
                        .map_or_else(|| ty.zero(), |who| memory::get(world, who, memory))
                })
            }
            RiftQuery::SetWithin {
                of,
                memory,
                seconds,
            } => {
                let memory = *memory;
                let of = cx.actor(of.as_deref())?;
                let seconds = cx.expr(seconds, ValueType::Number)?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        memory::set_within(
                            world,
                            who,
                            memory,
                            Seconds(seconds(world, cx).as_num() as f32),
                        )
                    }))
                })
            }
            RiftQuery::OnQuest { of, quest } => {
                let quest = *quest;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        quest::log_of(world, who).is_some_and(|log| log.active(quest).is_some())
                    }))
                })
            }
            RiftQuery::QuestDone { of, quest } => {
                let quest = *quest;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        quest::log_of(world, who).is_some_and(|log| {
                            log.finished(quest)
                                .is_some_and(|done| done.result == quest::QuestResult::Completed)
                        })
                    }))
                })
            }
            RiftQuery::QuestCooling { of, quest } => {
                let quest = *quest;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        let today = world.resource::<WallClock>().day();
                        quest::log_of(world, who).is_some_and(|log| {
                            quest.get(world.resource::<Content>()).daily()
                                && log.done_for_now(world.resource::<Content>(), quest, today)
                        })
                    }))
                })
            }
            RiftQuery::TaskDone { of, quest, task } => {
                let quest = *quest;
                let of = cx.actor(of.as_deref())?;
                let index = quest
                    .get(cx.content)
                    .objectives()
                    .position(|objective| matches!(objective, Objective::Task { task: known, .. } if known == task))
                    .ok_or_else(|| {
                        ScriptFault::new(
                            "unknown task",
                            format!("{} has no task {task}", quest.key(cx.content)),
                        )
                    })?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        quest::log_of(world, who).is_some_and(|log| {
                            log.active(quest).is_some_and(|active| {
                                active.counts.get(index).is_some_and(|count| *count > 0)
                            })
                        })
                    }))
                })
            }
            RiftQuery::QuestResetsIn { of, quest } => {
                let quest = *quest;
                if !quest.get(cx.content).daily() {
                    return Err(ScriptFault::new(
                        "never resets",
                        format!("a daily quest, not {}", quest.key(cx.content)),
                    ));
                }
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Text(
                        character(world, cx, &of)
                            .and_then(|who| quest::log_of(world, who))
                            .and_then(|log| log.finished(quest))
                            .and_then(|done| done.resets_in(world.resource::<WallClock>().now))
                            .map(Seconds::in_words)
                            .unwrap_or_default(),
                    )
                })
            }
            RiftQuery::Inside { of, object } => {
                let object = object.clone();
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        actor(world, cx, &of)
                            .is_some_and(|who| object::inside(world, who, &object)),
                    )
                })
            }
            RiftQuery::InArea { of, area } => {
                let area = *area;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(actor(world, cx, &of).is_some_and(|who| {
                        world
                            .get::<AreaTag>(who)
                            .is_some_and(|tag| tag.area == area)
                    }))
                })
            }
            RiftQuery::KnowsArea { of, area } => {
                let area = *area;
                let of = cx.actor(of.as_deref())?;
                Box::new(move |world, cx| {
                    Value::Bool(character(world, cx, &of).is_some_and(|who| {
                        world
                            .get::<Discovered>(who)
                            .is_some_and(|known| known.knows(area))
                    }))
                })
            }
            RiftQuery::Nearest { among, npc, within } => {
                let (among, npc) = (*among, *npc);
                let within = cx.expr(within, ValueType::Number)?;
                Box::new(move |world, cx| {
                    let within = Tiles(within(world, cx).as_num() as f32);
                    Value::Actor(nearest(world, cx, among, npc, within))
                })
            }
            RiftQuery::Count { among, npc, within } => {
                let (among, npc) = (*among, *npc);
                let within = cx.expr(within, ValueType::Number)?;
                Box::new(move |world, cx| {
                    let within = Tiles(within(world, cx).as_num() as f32);
                    Value::Int(around(world, cx, among, npc, within).count() as i64)
                })
            }
            RiftQuery::Present { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| Value::Bool(actor(world, cx, &who).is_some()))
            }
            RiftQuery::Distance { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| {
                    let apart = cx
                        .owner
                        .zip(actor(world, cx, &who))
                        .and_then(|(owner, who)| distance(world, owner, who));
                    Value::Num(f64::from(apart.map_or(0.0, |apart| apart.0)))
                })
            }
            RiftQuery::Health { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| {
                    Value::Num(
                        actor(world, cx, &who)
                            .map_or(0.0, |who| f64::from(stat::current_health(world, who))),
                    )
                })
            }
            RiftQuery::IsNpc { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        actor(world, cx, &who).is_some_and(|who| world.get::<Npc>(who).is_some()),
                    )
                })
            }
            RiftQuery::Subject => Box::new(|_, cx| Value::Actor(cx.subject)),
            RiftQuery::Target => Box::new(|world, cx| {
                Value::Actor(cx.owner.and_then(|owner| body::target(world, owner)))
            }),
            RiftQuery::Walking { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        actor(world, cx, &who).is_some_and(|who| movement::moving(world, who)),
                    )
                })
            }
            RiftQuery::Hostile { actor: who } => {
                let who = cx.expr(who, ValueType::Actor)?;
                Box::new(move |world, cx| {
                    Value::Bool(
                        actor(world, cx, &who).is_some_and(|who| {
                            world.get::<Attitude>(who) == Some(&Attitude::Hostile)
                        }),
                    )
                })
            }
            RiftQuery::GroundItems { item, at, within } => {
                let item = *item;
                let at = cx.opt_expr(at.as_deref(), ValueType::Actor)?;
                let within = cx.expr(within, ValueType::Number)?;
                Box::new(move |world, cx| {
                    let within = Tiles(within(world, cx).as_num() as f32);
                    let center = match &at {
                        Some(at) => {
                            actor(world, cx, at).and_then(|who| movement::position(world, who))
                        }
                        None => cx.owner.and_then(|owner| movement::position(world, owner)),
                    };
                    let Some(center) = center else {
                        return Value::Int(0);
                    };
                    let area = cx
                        .owner
                        .and_then(|owner| world.get::<AreaTag>(owner).map(|tag| tag.area));
                    let mut count = 0;
                    for entity in world.iter_entities() {
                        let Some(drop) = entity.get::<DroppedItem>() else {
                            continue;
                        };
                        if drop.item != item {
                            continue;
                        }
                        if area.is_some() && entity.get::<AreaTag>().map(|tag| tag.area) != area {
                            continue;
                        }
                        let Some(at) = entity.get::<Position>() else {
                            continue;
                        };
                        if at.pos.distance(center) <= within {
                            count += i64::from(drop.count);
                        }
                    }
                    Value::Int(count)
                })
            }
            RiftQuery::SkillTargets => {
                let skill = skill_of(cx)?;
                Box::new(move |world, cx| Value::Actors(skill::targets(world, cx, skill)))
            }
            RiftQuery::SkillArea => {
                let skill = skill_of(cx)?;
                Box::new(move |world, cx| Value::Area(skill::footprint(world, cx, skill)))
            }
            RiftQuery::AimedPoint => {
                let skill = skill_of(cx)?;
                Box::new(move |world, cx| Value::Point(skill::aimed_point(world, cx, skill)))
            }
        })
    }
}

fn status_of(cx: &Compiler<RiftScripting>) -> Result<StatusEffectId, ScriptFault> {
    cx.owner_row()
        .filter(|row| row.kind == OwnerKind::StatusEffect)
        .map(|row| row.id::<StatusEffectDef>())
        .ok_or_else(|| {
            ScriptFault::new(
                "wrong owner",
                "status_power reads the status whose script this is; only a status effect's script may ask",
            )
        })
}

fn skill_of(cx: &Compiler<RiftScripting>) -> Result<SkillId, ScriptFault> {
    cx.owner_row()
        .filter(|row| row.kind == OwnerKind::Skill)
        .map(|row| row.id::<SkillDef>())
        .ok_or_else(|| {
            ScriptFault::new(
                "wrong owner",
                "only a skill's own script knows what it was aimed at",
            )
        })
}

pub fn actor(world: &World, cx: &FlowCx, eval: &Eval) -> Option<Entity> {
    let who = eval(world, cx).as_actor()?;
    (world.get_entity(who).is_ok() && !stat::is_dead(world, who)).then_some(who)
}

pub fn actors(world: &World, cx: &FlowCx, eval: &Eval) -> Vec<Entity> {
    eval(world, cx)
        .as_actors()
        .into_iter()
        .filter(|who| world.get_entity(*who).is_ok() && !stat::is_dead(world, *who))
        .collect()
}

pub fn character(world: &World, cx: &FlowCx, eval: &Eval) -> Option<Entity> {
    actor(world, cx, eval).filter(|who| is_character(world, *who))
}

pub fn is_character(world: &World, who: Entity) -> bool {
    world.get::<PlayerOwner>(who).is_some()
}

fn distance(world: &World, from: Entity, to: Entity) -> Option<Tiles> {
    Some(movement::position(world, from)?.distance(movement::position(world, to)?))
}

pub fn nearest(
    world: &World,
    cx: &FlowCx,
    among: Among,
    npc: Option<NpcId>,
    within: Tiles,
) -> Option<Entity> {
    around(world, cx, among, npc, within)
        .min_by(|(a, da), (b, db)| da.0.total_cmp(&db.0).then(a.cmp(b)))
        .map(|(entity, _)| entity)
}

pub fn around<'w>(
    world: &'w World,
    cx: &FlowCx,
    among: Among,
    npc: Option<NpcId>,
    within: Tiles,
) -> impl Iterator<Item = (Entity, Tiles)> + 'w {
    let scan = Scan::of(world, cx, among, within);
    let candidates: &'w [Listed] = match &scan {
        Some(scan) => scan.roster.among(among),
        None => &[],
    };
    candidates.iter().filter_map(move |listed| {
        let scan = scan.as_ref()?;
        let fits = listed.entity != scan.owner
            && listed.area == scan.area
            && !listed.dead
            && !listed.held
            && match among {
                Among::Characters => listed.character,
                Among::Npcs => listed.npc.is_some(),
                Among::Anyone => true,
                Among::Attackers | Among::PackAttackers => scan.attackers.contains(&listed.entity),
            }
            && npc.is_none_or(|npc| listed.npc == Some(npc));
        if !fits {
            return None;
        }
        let distance = scan.from.distance(listed.at);
        if distance > within {
            return None;
        }
        let seen = !(scan.owner_presence || listed.presence)
            || visibility::coexist(world, scan.owner, listed.entity);
        let exposed = scan.shelter.is_none_or(|area| !area.safe(listed.at));
        (seen && exposed).then_some((listed.entity, distance))
    })
}

struct Scan<'w> {
    roster: &'w TickRoster,
    owner: Entity,
    from: Pos<Tiles>,
    area: AreaId,
    shelter: Option<&'w Area>,
    attackers: Vec<Entity>,
    owner_presence: bool,
}

impl<'w> Scan<'w> {
    fn of(world: &'w World, cx: &FlowCx, among: Among, within: Tiles) -> Option<Scan<'w>> {
        let owner = cx.owner?;
        let from = movement::position(world, owner)?;
        let area = world.get::<AreaTag>(owner)?.area;
        let roster = world.get_resource::<TickRoster>()?;
        if !roster.may_reach(among, from, within) {
            return None;
        }
        let shelter =
            area::wild(world, owner).then(|| area::load(world.resource::<Content>(), area));
        let attackers: Vec<Entity> = match among {
            Among::Attackers => world
                .get::<Attackers>(owner)
                .map(|attackers| attackers.ids.clone())
                .unwrap_or_default(),
            Among::PackAttackers => {
                let pack = world.get::<Pack>(owner).copied();
                roster
                    .iter()
                    .filter(|listed| listed.npc.is_some() && listed.area == area)
                    .filter(|listed| world.get::<Pack>(listed.entity).copied() == pack)
                    .flat_map(|listed| {
                        world
                            .get::<Attackers>(listed.entity)
                            .map(|attackers| attackers.ids.clone())
                            .unwrap_or_default()
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        Some(Scan {
            roster,
            owner,
            from,
            area,
            shelter,
            attackers,
            owner_presence: world.get::<Presence>(owner).is_some(),
        })
    }
}
