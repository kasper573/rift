use serde::Deserialize;

use super::{Item, UseCtx};
use crate::systems::equipment::{Requirement, SlotId};

#[derive(Deserialize)]
pub struct EquipmentItem {
    pub slot: SlotId,
    #[serde(default, deserialize_with = "crate::systems::equipment::req::parse")]
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
