//! Items: the [`ItemDef`] catalog, the replicated [`Inventory`] a character carries, and the
//! use-item request/consumed messages with the server system that applies them.

use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::assets;
use crate::core::table::{self, Content, Id};
use crate::systems::sfx::SfxId;

use crate::systems::combat::{Vitals, is_dead};
use crate::systems::player::sender_player;
use crate::systems::visibility::seen_by;
use bevy_ecs::message::Messages;
use bevy_ecs::world::World;
use bevy_replicon::prelude::{FromClient, SendTargets, ToClients};

const FILE: &str = "item_table.json";

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Inventory>()
        .add_client_message::<UseItemRequest>(Channel::Ordered)
        .add_mapped_server_message::<ItemConsumed>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub items: Vec<Id<ItemDef>>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UseItemRequest {
    pub slot: u32,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: Id<ItemDef>,
    #[entities]
    pub actor: Entity,
}

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

impl Content for ItemDef {
    fn table() -> &'static [ItemDef] {
        items()
    }
    fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Consumable { health_bonus: f32 },
    Resource,
    Equipment,
}

pub struct Icon(pub String);

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        assets::find(assets::ICONS, &format!("{name}.png"))
            .map(Icon)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown icon '{name}'")))
    }
}

pub fn items() -> &'static [ItemDef] {
    static ITEMS: OnceLock<Vec<ItemDef>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let items: Vec<ItemDef> = table::load(FILE);
        table::unique_ids(items.iter().map(|item| item.id.as_str()), FILE);
        items
    })
}

pub fn use_item(world: &mut World) {
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
        match slotted.get().kind {
            ItemKind::Consumable { health_bonus } => {
                if let Some(mut vitals) = world.get_mut::<Vitals>(entity) {
                    vitals.heal(health_bonus);
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
