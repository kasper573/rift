use serde::Deserialize;

use super::{Item, UseCtx};
use crate::systems::equipment::{EquipSlot, RequirementKind};

/// Wearable gear: using it from the inventory equips it (if its requirements pass).
#[derive(Deserialize)]
pub struct EquipmentItem {
    pub slot: EquipSlot,
    #[serde(default)]
    pub requirements: Vec<RequirementKind>,
}

impl Item for EquipmentItem {
    fn use_from(&self, ctx: &mut UseCtx) {
        ctx.equip(self.slot, &self.requirements);
    }
    fn carried(&self) -> bool {
        false
    }
}
