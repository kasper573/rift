use serde::Deserialize;

use super::{Item, UseCtx};

/// A crafting/loot material: inert when used, but its effects apply while it is carried.
#[derive(Deserialize)]
pub struct ResourceItem {}

impl Item for ResourceItem {
    fn use_from(&self, _ctx: &mut UseCtx) {}
    fn carried(&self) -> bool {
        true
    }
}
