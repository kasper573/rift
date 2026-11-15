//! A tray of item slots a player fills from their bag: a prompt's, a message's attachments.
//! What is placed in a tray stays in the bag, and is known to this client alone until whoever
//! owns the tray sends it.

use bevy::prelude::*;
use ui::drag;
use ui::egui::{self, Sense, Ui, Vec2};

use super::widget::{SLOT, SlotLook, slot};
use super::{Inventory, ItemId, ItemStack, card};
use crate::core::content::Content;
use crate::systems::hud::{self, Carried};

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

/// What the player did to a tray's slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayDone {
    /// Clicked the slot at this index, to take back what it holds.
    Clicked(usize),
    /// Dropped what they carried from this slot of the bag onto the tray.
    Dropped(u32),
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

    /// Places one of the item in `bag`'s slot `slot`, or with `whole` all of that slot that is
    /// not placed yet: on the stack of it already there, else in the next empty slot of the
    /// `slots` the tray has. Whether anything was placed.
    pub fn place(&mut self, bag: &Inventory, slot: u32, whole: bool, slots: usize) -> bool {
        let Some(&stack) = bag.slots.get(slot as usize) else {
            return false;
        };
        let free = bag.count(stack.item).saturating_sub(self.count(stack.item));
        let count = if whole {
            stack.count.min(free)
        } else {
            free.min(1)
        };
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

    /// Takes back what the slot `index` holds.
    pub fn take_back(&mut self, index: usize) {
        if index < self.placed.len() {
            self.placed.remove(index);
        }
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

/// The slots of `tray`, `count` of them: every placed stack in its slot, each named `named` and
/// its count and item to whoever reads the interface. What the player did to them.
pub fn tray_slots(
    ui: &mut Ui,
    world: &mut World,
    tray: &ItemTray,
    count: u32,
    named: &str,
) -> Option<TrayDone> {
    let content = world.resource::<Content>().clone();
    let mut clicked = None;
    let shown = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
            ui.horizontal_wrapped(|ui| {
                for index in 0..count as usize {
                    let Some(placed) = tray.placed.get(index) else {
                        slot(ui, world, SlotLook::EMPTY, Sense::hover()).widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Empty slot")
                        });
                        continue;
                    };
                    let def = placed.stack.item.get(&content);
                    let look = SlotLook {
                        count: placed.stack.count,
                        refused: placed.short,
                        ..SlotLook::of(&content, placed.stack.item)
                    };
                    let response = slot(ui, world, look, Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("{named}: {} {}", placed.stack.count, def.name),
                        )
                    });
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    let lines = if placed.short {
                        vec!["You no longer carry this".to_owned()]
                    } else {
                        vec!["Click to take it back".to_owned()]
                    };
                    card::options(world, &response, &[placed.stack.item]);
                    hud::tooltip(&response, world, &def.name, &lines, None);
                }
            });
        })
        .response;
    if let Some(index) = clicked {
        return Some(TrayDone::Clicked(index));
    }
    match drag::dropped::<Carried>(&shown) {
        Some(Carried::BagSlot(slot)) => Some(TrayDone::Dropped(slot)),
        _ => None,
    }
}

/// How tall a tray of `slots` slots stands in a panel `width` wide.
pub fn slots_height(slots: u32, width: f32) -> f32 {
    let across = (((width + SLOT_GAP) / (SLOT + SLOT_GAP)).floor() as u32).max(1);
    let rows = slots.div_ceil(across).max(1) as f32;
    rows * SLOT + (rows - 1.0) * SLOT_GAP
}

pub fn slots_width(across: u32) -> f32 {
    across as f32 * SLOT + across.saturating_sub(1) as f32 * SLOT_GAP
}

const SLOT_GAP: f32 = 6.0;
