mod head;
mod offhand;
mod weapon;

pub use head::HeadSlot;
pub use offhand::OffhandSlot;
pub use weapon::WeaponSlot;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub trait Slot: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str;
}

inventory::collect!(&'static dyn Slot);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(&'static str);

impl SlotId {
    pub fn name(self) -> &'static str {
        self.0
    }
    pub fn label(self) -> &'static str {
        def(self).label()
    }
}

impl<S: Slot> From<S> for SlotId {
    fn from(slot: S) -> SlotId {
        SlotId(slot.name())
    }
}

impl Default for SlotId {
    fn default() -> SlotId {
        all().next().expect("at least one registered slot")
    }
}

pub fn all() -> impl Iterator<Item = SlotId> {
    let mut ids: Vec<SlotId> = inventory::iter::<&'static dyn Slot>()
        .map(|slot| SlotId(slot.name()))
        .collect();
    ids.sort();
    ids.into_iter()
}

fn def(id: SlotId) -> &'static dyn Slot {
    inventory::iter::<&'static dyn Slot>()
        .copied()
        .find(|slot| slot.name() == id.0)
        .expect("a registered slot")
}

impl Serialize for SlotId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SlotId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        all()
            .find(|slot| slot.0 == name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown slot '{name}'")))
    }
}
