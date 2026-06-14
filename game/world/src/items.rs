use std::sync::OnceLock;

use serde::{Deserialize, Deserializer};

use crate::actors::SfxId;
use crate::assets;
use crate::protocol::ItemId;
use crate::table;

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

/// An icon asset's root-relative path, validated to exist when the table loads.
pub struct Icon(pub String);

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        assets::find(assets::ICONS, &format!("{name}.png"))
            .map(Icon)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown icon '{name}'")))
    }
}

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
