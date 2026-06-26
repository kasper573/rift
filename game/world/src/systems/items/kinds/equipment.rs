use serde::Deserialize;

use super::Kind;
use crate::systems::equipment::{EquipSlot, RequirementKind};
use crate::systems::items::UseCtx;

/// Wearable gear: using it from the inventory equips it (if its requirements pass).
#[derive(Deserialize)]
pub struct Equipment {
    pub slot: EquipSlot,
    #[serde(default)]
    pub requirements: Vec<RequirementKind>,
}

impl Kind for Equipment {
    fn use_from(&self, ctx: &mut UseCtx) {
        ctx.equip(self.slot, &self.requirements);
    }
    fn carried(&self) -> bool {
        false
    }
}
