//! A tray of item slots a player fills from their bag: a prompt's, a message's attachments, a
//! new listing's goods, a sale. What is placed in a tray stays in the bag, and is known to this
//! client alone until whoever owns the tray sends it.

use super::{Inventory, ItemId, ItemStack, SlotStack};

/// What a player has placed in a tray, slot by slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemTray {
    placed: Vec<Placed>,
}

/// A stack placed in a slot of a tray; `short` once the bag no longer covers what was placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub stack: ItemStack,
    pub short: bool,
}

impl ItemTray {
    pub fn placed(&self) -> &[Placed] {
        &self.placed
    }

    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// Every stack placed that the bag still covers.
    pub fn covered(&self) -> Vec<ItemStack> {
        self.placed
            .iter()
            .filter(|placed| !placed.short)
            .map(|placed| placed.stack)
            .collect()
    }

    pub fn holds(&self, item: ItemId) -> bool {
        self.placed.iter().any(|placed| placed.stack.item == item)
    }

    /// Places `count` of the item in `bag`'s slot `slot`, no more than the bag holds that is not
    /// placed yet: on the stack of it already there, else in the next empty slot of the `slots`
    /// the tray has. Whether anything was placed.
    pub fn place(&mut self, bag: &Inventory, slot: u32, count: u32, slots: usize) -> bool {
        let Some(&stack) = bag.slots.get(slot as usize) else {
            return false;
        };
        let free = bag.count(stack.item).saturating_sub(self.count(stack.item));
        let count = count.min(stack.count).min(free);
        let room = self.placed.len() < slots || self.holds(stack.item);
        if count == 0 || !room {
            return false;
        }
        self.put(ItemStack::new(stack.item, count));
        true
    }

    /// Places `stack` whatever is there: on the stack of its item, else in a slot of its own.
    pub fn put(&mut self, stack: ItemStack) {
        match self
            .placed
            .iter_mut()
            .find(|placed| placed.stack.item == stack.item)
        {
            Some(placed) => placed.stack.count += stack.count,
            None => self.placed.push(Placed {
                stack,
                short: false,
            }),
        }
    }

    /// Takes back `count` of what the slot `index` holds. Whether anything was taken back.
    pub fn take_back(&mut self, index: usize, count: u32) -> bool {
        let Some(placed) = self.placed.get_mut(index) else {
            return false;
        };
        placed.stack.count = placed.stack.count.saturating_sub(count);
        if placed.stack.count == 0 {
            self.placed.remove(index);
        }
        count > 0
    }

    pub fn clear(&mut self) {
        self.placed.clear();
    }

    /// How many of the stack in `bag`'s slot `slot` are placed: what is placed of an item is
    /// counted off its stacks from the last one back, as the bag gives them up.
    pub fn aside(&self, bag: &Inventory, slot: u32, stack: ItemStack) -> u32 {
        let placed = self.count(stack.item);
        if placed == 0 {
            return 0;
        }
        let later: u32 = bag
            .slots
            .iter()
            .skip(slot as usize + 1)
            .filter(|other| other.item == stack.item)
            .map(|other| other.count)
            .sum();
        placed.saturating_sub(later).min(stack.count)
    }

    /// What is placed, taken from `bag` slot by slot, the last of an item's stacks first.
    pub fn from_slots(&self, bag: &Inventory) -> Vec<SlotStack> {
        bag.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, &stack)| {
                let slot = slot as u32;
                let count = self.aside(bag, slot, stack);
                (count > 0).then(|| SlotStack {
                    slot,
                    stack: ItemStack::new(stack.item, count),
                })
            })
            .collect()
    }

    /// Keeps what is placed within what `bag` holds, marking what it no longer covers.
    pub fn keep_within(&mut self, bag: &Inventory) {
        let mut left: Vec<(ItemId, u32)> = self
            .placed
            .iter()
            .map(|placed| (placed.stack.item, bag.count(placed.stack.item)))
            .collect();
        for placed in &mut self.placed {
            let have = left
                .iter_mut()
                .find(|(item, _)| *item == placed.stack.item)
                .map_or(0, |(_, have)| std::mem::take(have));
            if have < placed.stack.count {
                placed.stack.count = have;
                placed.short = true;
            }
        }
    }

    /// Lets go of what the bag stopped covering, so what is left is all there to send.
    pub fn drop_short(&mut self) {
        self.placed.retain(|placed| placed.stack.count > 0);
        for placed in &mut self.placed {
            placed.short = false;
        }
    }

    fn count(&self, item: ItemId) -> u32 {
        self.placed
            .iter()
            .filter(|placed| placed.stack.item == item)
            .map(|placed| placed.stack.count)
            .sum()
    }
}
