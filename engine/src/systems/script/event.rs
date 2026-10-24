use bevy_ecs::entity::Entity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString, IntoStaticStr};

use crate::core::content::serial;
use crate::core::math::Pos;
use crate::core::script::{EventDef, ScriptFault, Value, ValueType};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::attention::AttentionId;
use crate::systems::interact::Verb;
use crate::systems::item::ItemId;
use crate::systems::npc::NpcId;
use crate::systems::script::{RiftScripting, RunnerKind};

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Characters,
    Npcs,
    Anyone,
}

#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "event", rename_all = "snake_case")]
#[strum_discriminants(name(EventKind), derive(Hash, EnumString, IntoStaticStr))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum RiftEvent {
    Started,
    Run,
    Entered {
        by: Who,
    },
    Left {
        by: Who,
    },
    StoodOn {
        by: Who,
        object: String,
    },
    Discovered,
    Died {
        by: Who,
    },
    LeveledUp,
    Interacted {
        verb: Verb,
        reach: Meters,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        marks: Vec<AttentionId>,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        news: bool,
    },
    Heard {
        words: Vec<String>,
        within: Meters,
    },
    CameNear {
        within: Meters,
        by: Who,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        npc: Option<NpcId>,
    },
    Attacked,
    Defeated,
    Used,
    PickedUp,
    Activated,
    Deactivated,
    Interval {
        seconds: Seconds,
    },
    Accepted,
    Completed,
    Failed,
    Bought {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
    },
    Sold {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemId>,
    },
}

pub(crate) const PLACED: &[RunnerKind] = &[
    RunnerKind::Object,
    RunnerKind::Npc,
    RunnerKind::Area,
    RunnerKind::Blueprint,
];
pub(crate) const OBJECT_OR_AREA: &[RunnerKind] = &[RunnerKind::Object, RunnerKind::Area];
pub(crate) const OBJECT_OR_NPC: &[RunnerKind] = &[RunnerKind::Object, RunnerKind::Npc];
pub(crate) const AREA: &[RunnerKind] = &[RunnerKind::Area];
pub(crate) const NPC: &[RunnerKind] = &[RunnerKind::Npc];
pub(crate) const ITEM: &[RunnerKind] = &[RunnerKind::Item];
pub(crate) const ITEM_OR_SKILL: &[RunnerKind] = &[RunnerKind::Item, RunnerKind::Skill];
pub(crate) const ASPECT: &[RunnerKind] = &[
    RunnerKind::Item,
    RunnerKind::Skill,
    RunnerKind::StatusEffect,
];
pub(crate) const QUEST: &[RunnerKind] = &[RunnerKind::Quest];
pub(crate) const SHOP: &[RunnerKind] = &[RunnerKind::Shop];
pub(crate) const ANY: &[RunnerKind] = &[RunnerKind::Any];

const TRADE_FIELDS: &[(&str, ValueType)] =
    &[("item", ValueType::Integer), ("count", ValueType::Integer)];

/// What a use hands its script: the aimed actor, the aimed point and the rank used at. An item
/// raises it with none of them.
pub const USED_FIELDS: &[(&str, ValueType)] = &[
    ("target", ValueType::Actor),
    ("point", ValueType::Point),
    ("rank", ValueType::Integer),
];
pub const USED_TARGET: usize = 0;
pub const USED_POINT: usize = 1;
pub const USED_RANK: usize = 2;

/// A `Used` payload laid out as `USED_FIELDS` lists it.
pub fn used_payload(target: Option<Entity>, point: Option<Pos<Meters>>, rank: i64) -> Vec<Value> {
    let mut fields = vec![Value::Actor(None); USED_FIELDS.len()];
    fields[USED_TARGET] = Value::Actor(target);
    fields[USED_POINT] = Value::Point(point);
    fields[USED_RANK] = Value::Int(rank);
    fields
}

impl EventDef<RiftScripting> for RiftEvent {
    type Kind = EventKind;

    fn kind(&self) -> EventKind {
        self.into()
    }

    fn owners(&self) -> &'static [RunnerKind] {
        match self {
            RiftEvent::Started => PLACED,
            RiftEvent::Run => ANY,
            RiftEvent::Entered { .. } | RiftEvent::Left { .. } => OBJECT_OR_AREA,
            RiftEvent::StoodOn { .. } => NPC,
            RiftEvent::Discovered | RiftEvent::Died { .. } | RiftEvent::LeveledUp => AREA,
            RiftEvent::Interacted { .. } | RiftEvent::Heard { .. } => OBJECT_OR_NPC,
            RiftEvent::CameNear { .. } | RiftEvent::Attacked | RiftEvent::Defeated => NPC,
            RiftEvent::Used => ITEM_OR_SKILL,
            RiftEvent::PickedUp => ITEM,
            RiftEvent::Activated | RiftEvent::Deactivated | RiftEvent::Interval { .. } => ASPECT,
            RiftEvent::Accepted | RiftEvent::Completed | RiftEvent::Failed => QUEST,
            RiftEvent::Bought { .. } | RiftEvent::Sold { .. } => SHOP,
        }
    }

    fn has_subject(&self) -> bool {
        !matches!(self, RiftEvent::Started)
    }

    fn is_run(&self) -> bool {
        matches!(self, RiftEvent::Run)
    }

    fn fields(&self) -> &'static [(&'static str, ValueType)] {
        match self {
            RiftEvent::Bought { .. } | RiftEvent::Sold { .. } => TRADE_FIELDS,
            RiftEvent::Used => USED_FIELDS,
            _ => &[],
        }
    }

    fn check(&self) -> Result<(), ScriptFault> {
        match self {
            RiftEvent::Interval { seconds } if *seconds < SHORTEST_INTERVAL => {
                Err(ScriptFault::new(
                    "interval too short",
                    format!("{} seconds or more, got {}", SHORTEST_INTERVAL.0, seconds.0),
                ))
            }
            _ => Ok(()),
        }
    }
}

/// An interval fires at most this often, so no script can beat a tick.
pub const SHORTEST_INTERVAL: Seconds = Seconds(0.1);

impl EventKind {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

impl Who {
    pub fn admits(self, is_npc: bool) -> bool {
        match self {
            Who::Characters => !is_npc,
            Who::Npcs => is_npc,
            Who::Anyone => true,
        }
    }
}
