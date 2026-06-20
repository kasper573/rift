use std::sync::OnceLock;

use serde::{Deserialize, Deserializer};

use crate::actors::SfxId;
use crate::assets;
use crate::table::{self, Content};

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
