use serde::Deserialize;

use super::{Chance, Grant, GrantCtx};
use crate::core::table::Id;
use crate::systems::items::ItemDef;

/// Drops an item, optionally gated by a percentage `chance` (absent = a guaranteed drop).
#[derive(Deserialize)]
pub struct Item {
    #[serde(deserialize_with = "Id::<ItemDef>::deserialize_named")]
    pub item: Id<ItemDef>,
    pub chance: Option<Chance>,
}

impl Grant for Item {
    fn grant(&self, ctx: &mut GrantCtx) {
        if self
            .chance
            .is_none_or(|percent| ctx.rng.unit() * 100.0 < percent.0)
        {
            ctx.drops.push((self.item, ctx.amount));
        }
    }
}
