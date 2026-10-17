use engine::core::content::{ContentRow, Fixture};
use engine::systems::area::world_map::{WorldMap, WorldMapDef};

use crate::views::character;
use crate::views::script::ScriptOwner;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EditorTab {
    World,
    Row { table: String, key: String },
    Rows { table: String, keys: Vec<String> },
    Area { key: String },
    Blueprint { key: String },
    Script(ScriptOwner),
    Model { key: String },
    Tileset { key: String },
    Spreadsheet { table: String },
}

/// What a tab's context menu asked to close, done once the dock is no longer being drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum CloseTabs {
    All,
    Others(EditorTab),
    ToTheRight(EditorTab),
}

impl EditorTab {
    pub fn title(&self) -> String {
        match self {
            EditorTab::World => "World".to_owned(),
            EditorTab::Row { table, .. } if table == character::TABLE => {
                "Test characters".to_owned()
            }
            EditorTab::Row { table, key } => format!("{table}/{key}"),
            EditorTab::Rows { table, keys } => format!("{} {table}s", keys.len()),
            EditorTab::Area { key } => format!("area/{key}"),
            EditorTab::Blueprint { key } => format!("blueprint/{key}"),
            EditorTab::Script(owner) => owner.title(),
            EditorTab::Model { key } => format!("model/{key}"),
            EditorTab::Tileset { key } => format!("tileset/{key}"),
            EditorTab::Spreadsheet { table } => format!("{table} table"),
        }
    }

    pub fn row(&self) -> Option<(String, String)> {
        match self {
            EditorTab::World => Some((
                WorldMapDef::TABLE.to_owned(),
                WorldMap::World.key().to_owned(),
            )),
            EditorTab::Row { table, key } => Some((table.clone(), key.clone())),
            EditorTab::Rows { .. } => None,
            EditorTab::Area { key } => Some(("area".to_owned(), key.clone())),
            EditorTab::Blueprint { key } => Some(("blueprint".to_owned(), key.clone())),
            EditorTab::Script(owner) => Some(owner.row()),
            EditorTab::Model { key } => Some(("model".to_owned(), key.clone())),
            EditorTab::Tileset { key } => Some(("tileset".to_owned(), key.clone())),
            EditorTab::Spreadsheet { .. } => None,
        }
    }

    pub fn is_row(&self, table: &str, key: &str) -> bool {
        match self {
            EditorTab::Script(owner) => owner.is_row(table, key),
            other => other.row().is_some_and(|(t, k)| t == table && k == key),
        }
    }

    pub fn for_row(table: &str, key: &str) -> EditorTab {
        match table {
            "area" => EditorTab::Area {
                key: key.to_owned(),
            },
            "blueprint" => EditorTab::Blueprint {
                key: key.to_owned(),
            },
            "model" => EditorTab::Model {
                key: key.to_owned(),
            },
            "tileset" => EditorTab::Tileset {
                key: key.to_owned(),
            },
            "script" => EditorTab::Script(ScriptOwner::Shared {
                key: key.to_owned(),
            }),
            WorldMapDef::TABLE => EditorTab::World,
            _ => EditorTab::Row {
                table: table.to_owned(),
                key: key.to_owned(),
            },
        }
    }
}
