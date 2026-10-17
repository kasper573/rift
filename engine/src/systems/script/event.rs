use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString, IntoStaticStr};

use crate::core::content::serial;
use crate::core::script::{EventDef, Name, ValueType};
use crate::core::tiling::Tiles;
use crate::systems::attention::AttentionId;
use crate::systems::interact::Verb;
use crate::systems::item::ItemId;
use crate::systems::npc::NpcId;
use crate::systems::script::{OwnerKind, RiftScripting};

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
        reach: Tiles,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        marks: Vec<AttentionId>,
        #[serde(default, skip_serializing_if = "serial::is_false")]
        news: bool,
    },
    Heard {
        words: Vec<String>,
        within: Tiles,
    },
    CameNear {
        within: Tiles,
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
        seconds: f32,
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

const PLACED: &[OwnerKind] = &[
    OwnerKind::Object,
    OwnerKind::Npc,
    OwnerKind::Area,
    OwnerKind::Blueprint,
];
const OBJECT_OR_AREA: &[OwnerKind] = &[OwnerKind::Object, OwnerKind::Area];
const OBJECT_OR_NPC: &[OwnerKind] = &[OwnerKind::Object, OwnerKind::Npc];
const AREA: &[OwnerKind] = &[OwnerKind::Area];
const NPC: &[OwnerKind] = &[OwnerKind::Npc];
const ITEM: &[OwnerKind] = &[OwnerKind::Item];
const ITEM_OR_SKILL: &[OwnerKind] = &[OwnerKind::Item, OwnerKind::Skill];
const ASPECT: &[OwnerKind] = &[OwnerKind::Item, OwnerKind::Skill, OwnerKind::StatusEffect];
const QUEST: &[OwnerKind] = &[OwnerKind::Quest];
const SHOP: &[OwnerKind] = &[OwnerKind::Shop];
const ANY: &[OwnerKind] = &[OwnerKind::Any];

const TRADE_FIELDS: &[(Name, ValueType)] =
    &[("item", ValueType::Integer), ("count", ValueType::Integer)];

/// What a use hands its script: the aimed actor, the aimed point and the rank used at. An item
/// raises it with none of them.
pub const USED_FIELDS: &[(Name, ValueType)] = &[
    ("target", ValueType::Actor),
    ("point", ValueType::Point),
    ("rank", ValueType::Integer),
];
pub const USED_TARGET: usize = 0;
pub const USED_POINT: usize = 1;
pub const USED_RANK: usize = 2;

impl EventDef<RiftScripting> for RiftEvent {
    type Kind = EventKind;

    fn kind(&self) -> EventKind {
        self.into()
    }

    fn owners(&self) -> &'static [OwnerKind] {
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

    fn fields(&self) -> &'static [(Name, ValueType)] {
        match self {
            RiftEvent::Bought { .. } | RiftEvent::Sold { .. } => TRADE_FIELDS,
            RiftEvent::Used => USED_FIELDS,
            _ => &[],
        }
    }
}

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
