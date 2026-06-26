//! Item kinds: one file per kind implementing [`Kind`], dispatched by [`ItemKind`] (`enum_dispatch`).
//! `use_item` calls [`Kind::use_from`] and the carried-effects source calls [`Kind::carried`]; adding
//! a kind is a new file plus a variant — neither path matches on a specific kind.

mod consumable;
mod equipment;
mod resource;

pub use consumable::Consumable;
pub use equipment::Equipment;
pub use resource::Resource;

use enum_dispatch::enum_dispatch;
use serde::Deserialize;

use super::UseCtx;

#[enum_dispatch]
pub trait Kind {
    /// Acts on the item when it is used from an inventory slot, via the high-level ops on [`UseCtx`].
    fn use_from(&self, ctx: &mut UseCtx);
    /// Whether the item's effects are active merely from being carried (true only for resources).
    fn carried(&self) -> bool;
}

#[enum_dispatch(Kind)]
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Consumable(Consumable),
    Resource(Resource),
    Equipment(Equipment),
}
