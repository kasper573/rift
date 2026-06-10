use std::sync::OnceLock;

use crate::core::actors::SfxId;
use rift::{Builder, Ctx, Entity, Wire};
use serde::{Deserialize, Deserializer};

use crate::core::assets;
use crate::core::protocol::{Inventory, ItemId, UseItemRequest, Vitals, is_dead};
use crate::core::table;
use crate::features::player::Players;

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

/// Broadcast to a shard's clients when one of its players consumes an item. Named for the fact,
/// not the reaction: the server reports what happened and each client decides how to respond — here,
/// by sounding the item's sfx, attenuated by where `actor` is relative to the listener.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: ItemId,
    pub actor: Entity,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Consumable { health_bonus: f32 },
    Resource,
    Equipment,
}

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        items()
            .iter()
            .position(|item| item.id == id)
            .map(|index| ItemId(index as u16))
            .ok_or_else(|| serde::de::Error::custom(format!("unknown item '{id}'")))
    }
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

pub fn feature(b: &mut Builder) {
    b.intent(use_item);
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
fn use_item(ctx: &mut Ctx) {
    for (client, req) in ctx.server.drain_events::<UseItemRequest>() {
        let Some(&entity) = ctx.res.get::<Players>().and_then(|p| p.0.get(&client)) else {
            continue;
        };
        let world = &mut ctx.server.world;
        if is_dead(world, entity) {
            continue;
        }
        let slot = req.slot as usize;
        let Some(slotted) = world
            .get::<Inventory>(entity)
            .and_then(|inventory| inventory.items.get(slot).copied())
        else {
            continue;
        };
        match item(slotted).kind {
            ItemKind::Consumable { health_bonus } => {
                world.modify::<Vitals>(entity, |v| {
                    v.health = (v.health + health_bonus).min(v.max);
                });
                world.modify::<Inventory>(entity, |inventory| {
                    inventory.items.remove(slot);
                });
            }
            ItemKind::Resource | ItemKind::Equipment => continue,
        }
        ctx.server.broadcast(&ItemConsumed {
            item: slotted,
            actor: entity,
        });
    }
}
