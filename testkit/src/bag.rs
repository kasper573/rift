use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::systems::equipment;
use engine::systems::item::{Inventory, ItemDef, ItemStack};

use crate::Sim;

impl Sim {
    /// Puts `count` of the item under `key` in `who`'s bag, the way a trade or a reward does.
    pub fn give(&mut self, who: Entity, key: &str, count: u32) {
        let item = self.id::<ItemDef>(key);
        let content = self.world.resource::<Content>().clone();
        self.world
            .get_mut::<Inventory>(who)
            .expect("a bag")
            .exchange(&content, &[], &[ItemStack::new(item, count)])
            .expect("room in the bag");
    }

    /// Puts the item under `key` in `who`'s bag and wears it from there, the way a player equips it,
    /// and lets a tick pass for what wearing it brings to settle.
    pub fn equip(&mut self, who: Entity, key: &str) {
        self.give(who, key, 1);
        let item = self.id::<ItemDef>(key);
        equipment::equip_from_bag(&mut self.world, who, item)
            .unwrap_or_else(|refused| panic!("{key} is worn: {refused}"));
        self.tick();
    }

    /// How many of the item under `key` lie in `who`'s bag.
    pub fn count(&self, who: Entity, key: &str) -> u32 {
        let item = self.id::<ItemDef>(key);
        self.world.get::<Inventory>(who).expect("a bag").count(item)
    }

    /// Where in `who`'s bag the item under `key` lies.
    pub fn slot_of(&self, who: Entity, key: &str) -> u32 {
        let item = self.id::<ItemDef>(key);
        self.world
            .get::<Inventory>(who)
            .expect("a bag")
            .slots
            .iter()
            .position(|stack| stack.item == item)
            .expect("in the bag") as u32
    }
}
