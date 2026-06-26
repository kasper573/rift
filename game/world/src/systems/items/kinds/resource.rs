use serde::Deserialize;

use super::Kind;
use crate::systems::items::UseCtx;

/// A crafting/loot material: inert when used, but its effects apply while it is carried.
#[derive(Deserialize)]
pub struct Resource {}

impl Kind for Resource {
    fn use_from(&self, _ctx: &mut UseCtx) {}
    fn carried(&self) -> bool {
        true
    }
}
