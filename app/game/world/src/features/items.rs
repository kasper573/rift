use std::sync::OnceLock;

use serde::{Deserialize, Deserializer};

use crate::core::actors::SfxId;
use crate::core::assets;
use crate::core::protocol::ItemId;
use crate::core::table;

const FILE: &str = "item_table.json";

#[derive(Deserialize)]
pub struct ItemDef {
    pub id: String,
    pub display_name: String,
    pub icon: Icon,
    #[serde(default)]
    pub sfx: Option<SfxId>,
    #[serde(flatten)]
    pub kind: ItemKind,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Consumable { health_bonus: f32 },
    Resource,
    Equipment,
}

/// An icon asset's name, resolved to the embedded PNG.
pub struct Icon(pub &'static [u8]);

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        assets::find(assets::ICONS, &format!("{name}.png"))
            .map(|(_, bytes)| Icon(bytes))
            .ok_or_else(|| serde::de::Error::custom(format!("unknown icon '{name}'")))
    }
}

/// Deserializes an [`ItemId`] from a content table's item id string.
pub fn item_by_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ItemId, D::Error> {
    let id = String::deserialize(deserializer)?;
    items()
        .iter()
        .position(|item| item.id == id)
        .map(|index| ItemId(index as u16))
        .ok_or_else(|| serde::de::Error::custom(format!("unknown item '{id}'")))
}

pub fn items() -> &'static [ItemDef] {
    static ITEMS: OnceLock<Vec<ItemDef>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let items: Vec<ItemDef> = table::load(FILE);
        table::unique_ids(items.iter().map(|item| item.id.as_str()), FILE);
        items
    })
}

pub fn item(id: ItemId) -> &'static ItemDef {
    &items()[id.0 as usize]
}

// The one place item effects apply, keyed by the item definition's kind.
#[cfg(feature = "host")]
pub fn use_item(world: &mut bevy_ecs::world::World) {
    use bevy_ecs::message::Messages;
    use bevy_replicon::prelude::{FromClient, SendTargets, ToClients};

    use crate::core::protocol::{Inventory, ItemConsumed, UseItemRequest, Vitals, is_dead};
    use crate::features::player::sender_player;
    use crate::features::visibility::seen_by;

    let requests: Vec<FromClient<UseItemRequest>> = world
        .resource_mut::<Messages<FromClient<UseItemRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if is_dead(world, entity) {
            continue;
        }
        let slot = request.message.slot as usize;
        let Some(slotted) = world
            .get::<Inventory>(entity)
            .and_then(|inventory| inventory.items.get(slot).copied())
        else {
            continue;
        };
        match item(slotted).kind {
            ItemKind::Consumable { health_bonus } => {
                if let Some(mut vitals) = world.get_mut::<Vitals>(entity) {
                    vitals.health = (vitals.health + health_bonus).min(vitals.max);
                }
                if let Some(mut inventory) = world.get_mut::<Inventory>(entity) {
                    inventory.items.remove(slot);
                }
            }
            ItemKind::Resource | ItemKind::Equipment => continue,
        }
        // Announced per beholder: a mapped message only decodes for clients that see the actor.
        for client in seen_by(world, entity) {
            world.write_message(ToClients {
                targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
                message: ItemConsumed {
                    item: slotted,
                    actor: entity,
                },
            });
        }
    }
}
