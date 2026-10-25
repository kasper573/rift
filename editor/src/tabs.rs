use engine::core::content::{
    ContentFile, Fixture, NOTE_EXTENSION, SETTINGS_FILE, fields_file, table_of,
};
use engine::systems::actor::model::ModelDef;
use engine::systems::area::AreaDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::world_map::{WorldMap, WorldMapDef};
use engine::systems::input::map::InputDef;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::script::ScriptDef;
use ui::egui_dock;
use ui::egui_dock::DockState;
use ui::glyph::Mark;

use crate::icons::{self, KindIcon};
use crate::project::files::content_holding;
use crate::views::script::ScriptRow;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EditorTab {
    World,
    Row { table: String, key: String },
    Rows { table: String, keys: Vec<String> },
    Area { key: String },
    Blueprint { key: String },
    Script(ScriptRow),
    Model { key: String },
    Prop { key: String },
    Spreadsheet { table: String },
    Game,
    Asset { path: String },
}

/// How long an opened tab stays: a preview gives way to the next preview, so browsing leaves one
/// tab behind, while a kept tab stays until it is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabOpening {
    Preview,
    Kept,
}

/// What a tab asked of the dock while the dock was being drawn, done once it no longer is.
#[derive(Clone, Debug, PartialEq)]
pub enum DockRequest {
    CloseAll,
    CloseOthers(EditorTab),
    CloseToTheRight(EditorTab),
    Focus(EditorTab),
}

impl EditorTab {
    pub fn title(&self) -> String {
        match self {
            EditorTab::World => "World".to_owned(),
            EditorTab::Row { table, .. } if table == CharacterSnapshot::TABLE => {
                "Test characters".to_owned()
            }
            EditorTab::Row { key, .. }
            | EditorTab::Area { key }
            | EditorTab::Blueprint { key }
            | EditorTab::Model { key }
            | EditorTab::Prop { key } => file_title(key).to_owned(),
            EditorTab::Rows { table, keys } => format!("{} {table}s", keys.len()),
            EditorTab::Script(owner) => owner.title(),
            EditorTab::Spreadsheet { table } => format!("{table} table"),
            EditorTab::Game => "game".to_owned(),
            EditorTab::Asset { path } => path.rsplit('/').next().unwrap_or(path).to_owned(),
        }
    }

    /// The icon of what the tab shows.
    pub fn icon(&self) -> KindIcon {
        match self {
            EditorTab::Game => Mark::Settings.into(),
            EditorTab::Spreadsheet { .. } => icons::TABLE,
            EditorTab::Rows { table, .. } => icons::of_table(table),
            EditorTab::Asset { path } if path.ends_with(&format!(".{NOTE_EXTENSION}")) => {
                icons::NOTE
            }
            EditorTab::Asset { path } => icons::of_media(path),
            other => other
                .row()
                .map_or(icons::TABLE, |(table, _)| icons::of_table(&table)),
        }
    }

    pub fn row(&self) -> Option<(String, String)> {
        match self {
            EditorTab::World => Some((WorldMapDef::TABLE.to_owned(), WorldMap::Game.path())),
            EditorTab::Row { table, key } => Some((table.clone(), key.clone())),
            EditorTab::Rows { .. } => None,
            EditorTab::Area { key } => Some((AreaDef::TABLE.to_owned(), key.clone())),
            EditorTab::Blueprint { key } => Some((BlueprintDef::TABLE.to_owned(), key.clone())),
            EditorTab::Script(owner) => Some(owner.row()),
            EditorTab::Model { key } => Some((ModelDef::TABLE.to_owned(), key.clone())),
            EditorTab::Prop { key } => Some((PropDef::TABLE.to_owned(), key.clone())),
            EditorTab::Spreadsheet { .. } | EditorTab::Game | EditorTab::Asset { .. } => None,
        }
    }

    /// Whether `file` holds any of what the tab shows, so changing it changes the tab.
    pub fn shows(&self, file: &str) -> bool {
        match self {
            EditorTab::Rows { keys, .. } => keys.iter().any(|key| holds(key, file)),
            EditorTab::Spreadsheet { table } => content_holding(file)
                .and_then(table_of)
                .is_some_and(|of| of == table),
            EditorTab::Game => file == SETTINGS_FILE,
            EditorTab::Asset { path } => file == path,
            other => other.row().is_some_and(|(_, key)| holds(&key, file)),
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
            AreaDef::TABLE => EditorTab::Area {
                key: key.to_owned(),
            },
            BlueprintDef::TABLE => EditorTab::Blueprint {
                key: key.to_owned(),
            },
            ModelDef::TABLE => EditorTab::Model {
                key: key.to_owned(),
            },
            PropDef::TABLE => EditorTab::Prop {
                key: key.to_owned(),
            },
            ScriptDef::TABLE => EditorTab::Script(ScriptRow::new(key)),
            WorldMapDef::TABLE => EditorTab::World,
            InputDef::TABLE => EditorTab::Spreadsheet {
                table: table.to_owned(),
            },
            _ => EditorTab::Row {
                table: table.to_owned(),
                key: key.to_owned(),
            },
        }
    }
}

/// Brings `tab` to the front of the dock. A preview takes the place of the tab previewed before
/// it, and opening a previewed tab to keep it keeps it where it is.
pub(crate) fn place(
    dock: &mut DockState<EditorTab>,
    preview: &mut Option<EditorTab>,
    tab: EditorTab,
    opening: TabOpening,
) {
    let previewed = preview
        .as_ref()
        .and_then(|previewed| dock.find_tab(previewed));
    if previewed.is_none() {
        *preview = None;
    }
    if let Some(path) = dock.find_tab(&tab) {
        let _ = dock.set_active_tab(path);
        if opening == TabOpening::Kept && preview.as_ref() == Some(&tab) {
            *preview = None;
        }
        return;
    }
    if opening == TabOpening::Preview {
        *preview = Some(tab.clone());
        if let Some(path) = previewed
            && let Ok(leaf) = dock.leaf_mut(path.node_path())
        {
            leaf.tabs[path.tab.0] = tab;
            let _ = dock.set_active_tab(path);
            return;
        }
    }
    if dock.iter_all_tabs().next().is_none() {
        *dock = DockState::new(vec![tab]);
        return;
    }
    if dock.focused_leaf().is_none() {
        let first = dock
            .main_surface()
            .iter()
            .enumerate()
            .find(|(_, node)| node.is_leaf())
            .map(|(index, _)| egui_dock::NodeIndex(index));
        if let Some(index) = first {
            dock.main_surface_mut().set_focused_node(index);
        }
    }
    dock.push_to_focused_leaf(tab);
}

pub(crate) fn serve(dock: &mut DockState<EditorTab>, request: DockRequest) {
    match request {
        DockRequest::CloseAll => dock.retain_tabs(|_| false),
        DockRequest::CloseOthers(kept) => dock.retain_tabs(|tab| *tab == kept),
        DockRequest::CloseToTheRight(from) => {
            let Some(path) = dock.find_tab(&from) else {
                return;
            };
            let Ok(leaf) = dock.leaf_mut(path.node_path()) else {
                return;
            };
            leaf.tabs.truncate(path.tab.0 + 1);
            leaf.active = path.tab;
        }
        DockRequest::Focus(tab) => {
            if let Some(path) = dock.find_tab(&tab) {
                dock.set_focused_node_and_surface(path.node_path());
            }
        }
    }
}

/// Whether `file` is the content file at `path` or one of its parts.
fn holds(path: &str, file: &str) -> bool {
    file == fields_file(path) || content_holding(file) == Some(path)
}

/// How a content file is named on its tab and in lists: its file name, without `.json`.
pub fn file_title(path: &str) -> &str {
    shown_path(path.rsplit('/').next().unwrap_or(path))
}

/// A path as the editor writes it for the user: a content file without the `.json` every one of
/// them ends in, since its type already says what it is.
pub fn shown_path(path: &str) -> &str {
    path.strip_suffix(".json").unwrap_or(path)
}
