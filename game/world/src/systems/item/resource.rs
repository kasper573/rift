use serde::Deserialize;

use super::{Item, UseCtx};

#[derive(Deserialize)]
pub struct ResourceItem {}

#[typetag::deserialize(name = "resource")]
impl Item for ResourceItem {
    fn use_from(&self, _ctx: &mut UseCtx) {}
    fn carried(&self) -> bool {
        true
    }
}
