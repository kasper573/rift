use serde::Deserialize;

use super::{Item, UseCtx};
use crate::systems::equipment::{EquipSlot, Requirement};

#[derive(Deserialize)]
pub struct EquipmentItem {
    pub slot: EquipSlot,
    #[serde(default)]
    pub requirements: Vec<Box<dyn Requirement>>,
}

#[typetag::deserialize(name = "equipment")]
impl Item for EquipmentItem {
    fn use_from(&self, ctx: &mut UseCtx) {
        ctx.equip(self.slot, &self.requirements);
    }
    fn carried(&self) -> bool {
        false
    }
}
