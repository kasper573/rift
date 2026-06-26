use serde::Deserialize;

use super::Kind;
use crate::core::time::Seconds;
use crate::systems::items::UseCtx;

/// A potion or food: heals, applies its timed effects, and is used up.
#[derive(Deserialize)]
pub struct Consumable {
    pub health_bonus: f32,
    #[serde(default)]
    pub duration: Seconds,
}

impl Kind for Consumable {
    fn use_from(&self, ctx: &mut UseCtx) {
        ctx.heal(self.health_bonus);
        ctx.consume();
        ctx.apply_effects(self.duration);
    }
    fn carried(&self) -> bool {
        false
    }
}
