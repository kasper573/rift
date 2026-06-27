use serde::Deserialize;

use super::{Item, UseCtx};
use crate::core::time::Seconds;

/// A potion or food: heals, applies its timed effects, and is used up.
#[derive(Deserialize)]
pub struct ConsumableItem {
    pub health_bonus: f32,
    #[serde(default)]
    pub duration: Seconds,
}

#[typetag::deserialize(name = "consumable")]
impl Item for ConsumableItem {
    fn use_from(&self, ctx: &mut UseCtx) {
        ctx.heal(self.health_bonus);
        ctx.consume();
        ctx.apply_effects(self.duration);
    }
    fn carried(&self) -> bool {
        false
    }
}
