use serde::Deserialize;

use super::{Chance, Grant, GrantCtx};
use crate::core::table::Id;
use crate::systems::item::ItemDef;

#[derive(Deserialize)]
pub struct ItemGrant {
    #[serde(deserialize_with = "Id::<ItemDef>::deserialize_named")]
    pub item: Id<ItemDef>,
    pub chance: Option<Chance>,
}

#[typetag::deserialize(name = "item")]
impl Grant for ItemGrant {
    fn grant(&self, ctx: &mut GrantCtx) {
        if self
            .chance
            .is_none_or(|percent| ctx.rng.unit() * 100.0 < percent.0)
        {
            ctx.drops.push((self.item, ctx.amount));
        }
    }
}
