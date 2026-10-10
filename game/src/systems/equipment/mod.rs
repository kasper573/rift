pub mod widget;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::Content;
use crate::data;
use crate::systems::effect::{self, Effect};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::sender_player;
use crate::systems::rule::Requirement;
use crate::systems::text::LineText;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered);
    effect::source(app, equipped);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipmentSlot, data::item::Id>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipmentSlot,
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    VariantArray,
    IntoStaticStr,
)]
pub enum EquipmentSlot {
    #[default]
    Weapon,
    Offhand,
    Head,
}

impl EquipmentSlot {
    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn all() -> &'static [EquipmentSlot] {
        EquipmentSlot::VARIANTS
    }
}

pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    into: EquipmentSlot,
    requirements: &'static [&'static dyn Requirement],
) {
    let content = world.resource::<Content>().clone();
    let Some(item) = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(inv_slot).map(|slot| slot.item))
    else {
        return;
    };
    if let Some(unmet) = requirements
        .iter()
        .find(|requirement| !requirement.met(world, player))
    {
        let refusal = format!(
            "{} needs {}",
            item.get(&content).display_name,
            unmet.describe(&content)
        );
        notification::notify(
            world,
            player,
            Notification::new(NotificationKind::error(), LineText::plain(refusal)),
        );
        return;
    }
    let occupant = world
        .get::<Equipment>(player)
        .and_then(|equipment| equipment.slots.get(&into).copied());
    if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
        inventory.slots.remove(inv_slot);
        if let Some(occupant) = occupant {
            inventory
                .exchange(&content, &[], &[ItemStack::new(occupant, 1)])
                .expect("the equipped item's slot was just freed");
        }
    }
    if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
        equipment.slots.insert(into, item);
    }
}

pub fn unequip(world: &mut World) {
    let content = world.resource::<Content>().clone();
    for request in crate::systems::requests::<UnequipRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let slot = request.message.slot;
        let Some(item) = world
            .get::<Equipment>(player)
            .and_then(|equipment| equipment.slots.get(&slot).copied())
        else {
            continue;
        };
        let stored = world
            .get_mut::<Inventory>(player)
            .map(|mut inventory| inventory.exchange(&content, &[], &[ItemStack::new(item, 1)]));
        match stored {
            Some(Ok(())) => {
                if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
                    equipment.slots.remove(&slot);
                }
            }
            Some(Err(refusal)) => notification::notify(
                world,
                player,
                Notification::new(
                    NotificationKind::error(),
                    LineText::plain(refusal.describe(&content)),
                ),
            ),
            None => {}
        }
    }
}

fn equipped(world: &World, entity: Entity) -> Vec<Effect> {
    let content = world.resource::<Content>();
    world
        .get::<Equipment>(entity)
        .map(|equipment| {
            equipment
                .slots
                .values()
                .flat_map(|item| item.get(content).effects.iter().copied())
                .collect()
        })
        .unwrap_or_default()
}

pub struct Wearing(pub data::item::Id);

impl Requirement for Wearing {
    fn met(&self, world: &World, player: Entity) -> bool {
        world
            .get::<Equipment>(player)
            .is_some_and(|equipment| equipment.slots.values().any(|&item| item == self.0))
    }

    fn describe(&self, content: &Content) -> String {
        format!("{} worn", self.0.get(content).display_name)
    }
}
