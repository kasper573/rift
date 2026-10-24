use std::path::PathBuf;

use engine::core::content::ContentFile;
use engine::systems::actor::model::ModelDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::tileset::TilesetDef;
use engine::systems::area::world_map::{Note, Region};
use engine::systems::area::{self, AreaDef};
use engine::systems::class::ClassDef;
use engine::systems::script::ScriptDef;
use serde::Deserialize;
use strum::VariantNames;

use crate::app::{BottomTab, Clipboard, Editor, ModalDialog, SceneClip};
use crate::import::ImportStep;
use crate::project::{ContentEdit, RowSelection};
use crate::tabs::EditorTab;
use crate::views::area::resize::ResizeDialog;
use crate::views::area::{self as area_view, blueprint};
use crate::views::character::CharacterSection;
use crate::views::class;
use crate::views::model::{self, ModelPage, busts};
use crate::views::script::{PendingScriptAction, ScriptRow};
use crate::views::tileset::{
    self, NewSurfaceDialog, NewTerrainSetDialog, SheetRect, TileBlueprintDialog, TileSheetDialog,
    TilesetMode, WalkBrush,
};
use crate::views::world::{
    self, ColorBy, ColorField, MapLens, MapLook, MapPart, MapRoute, OpenWorldMap, arrange,
};
use crate::{rows, shell};

#[derive(Clone, Debug, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SceneStep {
    Area(String),
    Blueprint(String),
    Resize(String),
    Row {
        table: String,
        key: String,
    },
    Rows {
        table: String,
        keys: Vec<String>,
    },
    Part {
        table: String,
        key: String,
        nodes: Vec<String>,
    },
    Fold {
        table: String,
        modules: Vec<usize>,
    },
    Unfold {
        table: String,
        modules: Vec<usize>,
    },
    /// Asks to delete `key`, pointing whatever names it at `repoint` when one is given.
    Delete {
        table: String,
        key: String,
        #[serde(default)]
        repoint: Option<String>,
    },
    Duplicate {
        table: String,
        key: String,
    },
    Clipboard {
        key: String,
        node: String,
    },
    Script(String),
    Node(Vec<String>),
    /// Links the output of `from` labelled `output` to `to`.
    Connect {
        from: String,
        output: String,
        to: String,
    },
    Bottom(BottomTab),
    Model(ModelStep),
    Tileset(TilesetStep),
    Sheet(SheetStep),
    World(WorldStep),
    Import(ImportStep),
    Section(CharacterSection),
    Tree {
        key: String,
        node: Option<String>,
    },
    Save,
    Dialog(DialogName),
    Game,
    Asset(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogName {
    Keys,
    Binaries,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStep {
    Open(String),
    Busts(Option<String>),
    Bottom(BottomTab),
    Drop(PathBuf),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilesetStep {
    Open(String),
    Mode(TilesetMode),
    Brush(SceneBrush),
    Select(TileSelection),
    Tile(u32),
    Set(String),
    Terrain(usize),
    Closeup(u32),
    Bottom(BottomTab),
    Zoom(f32),
    Dialog(TilesetDialogName),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneBrush {
    Walk(WalkBrush),
    Surface(String),
    NoSurface,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct TileSelection {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilesetDialogName {
    Blueprint,
    Surface,
    Set,
    None,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetStep {
    Open(String),
    Columns(Vec<String>),
    Sort {
        column: String,
        #[serde(default)]
        order: SortOrder,
    },
    Filter(String),
    Cell {
        key: String,
        column: String,
        #[serde(default)]
        editing: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldStep {
    Open,
    Lens(WorldLens),
    Color(Option<ColorField>),
    Line(String),
    Route(Option<MapRoute>),
    Look {
        area: String,
        zoom: f32,
    },
    Select(Vec<MapPart>),
    Layer(WorldLayer),
    Arrange,
    ArrangeApply,
    Region {
        name: String,
        cards: Vec<String>,
    },
    Note {
        text: String,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLens {
    Row { table: String, key: String },
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLayer {
    Residents,
    Populations,
    Givers,
}

pub fn open(editor: &mut Editor, steps: &[SceneStep]) {
    for step in steps {
        open_one(editor, step);
    }
}

fn open_one(editor: &mut Editor, step: &SceneStep) {
    match step {
        SceneStep::Area(key) => {
            editor.select(RowSelection::row(AreaDef::TABLE, key));
            editor.open_tab(EditorTab::Area { key: key.clone() });
        }
        SceneStep::Blueprint(key) => {
            editor.select(RowSelection::row(BlueprintDef::TABLE, key));
            editor.open_tab(EditorTab::Blueprint { key: key.clone() });
        }
        SceneStep::Part { table, key, nodes } => {
            editor.select(RowSelection {
                table: table.clone(),
                keys: vec![key.clone()],
                parts: nodes.clone(),
            });
            editor.open_tab(EditorTab::for_row(table, key));
        }
        SceneStep::Resize(key) => {
            let size = editor
                .project
                .as_ref()
                .and_then(|p| p.content.clone())
                .and_then(|c| c.by_path::<AreaDef>(key).map(|id| area::load(&c, id).size));
            editor.open_tab(EditorTab::Area { key: key.clone() });
            editor.dialogs.push(ModalDialog::Resize(ResizeDialog {
                bundle: key.clone(),
                width: size.map_or(0, |s| s.width as u32),
                height: size.map_or(0, |s| s.height as u32),
                anchor: (-1, -1),
                error: None,
            }));
        }
        SceneStep::Row { table, key } => {
            editor.open_row(table, key);
        }
        SceneStep::Rows { table, keys } => {
            editor.select(RowSelection::rows(table, keys.clone()));
            editor.open_tab(EditorTab::Rows {
                table: table.clone(),
                keys: keys.clone(),
            });
        }
        SceneStep::Fold { table, modules } => fold(editor, table, modules, true),
        SceneStep::Unfold { table, modules } => fold(editor, table, modules, false),
        SceneStep::Delete { key, repoint, .. } => {
            rows::delete_dialog(editor, vec![key.clone()]);
            if let (Some(target), Some(ModalDialog::Delete(dialog))) =
                (repoint, editor.dialogs.last_mut())
            {
                for group in &mut dialog.groups {
                    group.decision = rows::UsageDecision::Repoint(target.clone());
                }
            }
        }
        SceneStep::Duplicate { key, .. } => {
            if let Err(error) = rows::duplicate(editor, key) {
                editor.toast(error);
            }
        }
        SceneStep::Clipboard { key, node } => {
            let (Some(reader), Some(mut copied)) = (
                editor.reader(),
                area_view::nodes::node(&area_view::nodes::scene_of(editor, key), node).cloned(),
            ) else {
                return;
            };
            let origin =
                blueprint::selection_origin(std::slice::from_ref(&copied)).unwrap_or_default();
            blueprint::shift_tree(&mut copied, -origin.x, -origin.y);
            let Some(nodes) = area_view::nodes::CopiedNodes::of(&reader, &[copied]) else {
                return;
            };
            editor.clipboard = Some(Clipboard::Scene(SceneClip {
                from: key.clone(),
                nodes,
                count: 1,
                origin,
            }));
        }
        SceneStep::Script(key) => {
            editor.select(RowSelection::row(ScriptDef::TABLE, key));
            editor.open_tab(EditorTab::Script(ScriptRow::new(key.clone())));
        }
        SceneStep::Node(names) => {
            if let Some(script) = last_script(editor) {
                let parts = names.iter().map(|name| format!("node:{name}")).collect();
                editor.select(RowSelection::parts(ScriptDef::TABLE, &script.key, parts));
            }
        }
        SceneStep::Connect { from, output, to } => set_pending(
            editor,
            PendingScriptAction::Connect {
                from: from.clone(),
                output: output.clone(),
                to: to.clone(),
            },
        ),
        SceneStep::Bottom(tab) => editor.panels.bottom = *tab,
        SceneStep::Model(step) => model_step(editor, step),
        SceneStep::Tileset(step) => tileset_step(editor, step),
        SceneStep::Sheet(step) => sheet_step(editor, step),
        SceneStep::World(step) => {
            editor.open_tab(EditorTab::World);
            world_step(editor, step);
        }
        SceneStep::Import(step) => editor.jobs.script.push_back(step.clone()),
        SceneStep::Section(section) => editor.views.character.section = *section,
        SceneStep::Tree { key, node } => {
            editor.open_tab(EditorTab::for_row(ClassDef::TABLE, key));
            editor
                .views
                .class
                .pages
                .insert(key.clone(), class::ClassPage::Tree);
            class::select_node(editor, key, node.as_deref());
        }
        SceneStep::Save => {
            editor.save();
        }
        SceneStep::Game => {
            editor.select(RowSelection::game());
            editor.open_tab(EditorTab::Game);
        }
        SceneStep::Asset(path) => {
            editor.select(RowSelection::row("asset", path));
            editor.open_tab(EditorTab::Asset { path: path.clone() });
        }
        SceneStep::Dialog(DialogName::Keys) => editor.dialogs.push(ModalDialog::Keys),
        SceneStep::Dialog(DialogName::Binaries) => editor.dialogs.push(ModalDialog::Binaries),
    }
}

fn fold(editor: &mut Editor, table: &str, modules: &[usize], folded: bool) {
    for index in modules {
        let path = format!("{table}/modules/{index}");
        if folded {
            editor.form.unfolded.remove(&path);
            editor.form.folded.insert(path);
        } else {
            editor.form.folded.remove(&path);
            editor.form.unfolded.insert(path);
        }
    }
}

fn set_pending(editor: &mut Editor, action: PendingScriptAction) {
    if let Some(script) = last_script(editor) {
        editor.views.script.set_pending(&script, action);
    }
}

fn last_script(editor: &Editor) -> Option<ScriptRow> {
    editor
        .open_tabs
        .iter()
        .map(|(tab, _)| tab)
        .chain(editor.dock.iter_all_tabs().map(|(_, tab)| tab))
        .filter_map(|tab| match tab {
            EditorTab::Script(script) => Some(script.clone()),
            _ => None,
        })
        .last()
}

fn sheet_step(editor: &mut Editor, step: &SheetStep) {
    if let SheetStep::Open(table) = step {
        editor.open_tab(EditorTab::Spreadsheet {
            table: table.clone(),
        });
        return;
    }
    let pending = editor
        .open_tabs
        .iter()
        .rev()
        .find_map(|(tab, _)| match tab {
            EditorTab::Spreadsheet { table } => Some(table.clone()),
            _ => None,
        });
    let active = match shell::active_tab(editor) {
        Some(EditorTab::Spreadsheet { table }) => Some(table),
        _ => None,
    };
    let Some(table) = pending.or(active) else {
        return;
    };
    if let SheetStep::Cell {
        key,
        column,
        editing,
    } = step
    {
        let state = editor.views.spreadsheet.state_mut(&table);
        state.selected = Some((key.clone(), column.clone()));
        state.editing = editing.then(|| column.clone());
        editor.select(RowSelection {
            table: table.clone(),
            keys: vec![key.clone()],
            parts: vec![format!("#sheet/{column}")],
        });
        return;
    }
    let mut view = editor
        .settings
        .views
        .get(&table)
        .cloned()
        .unwrap_or_default();
    match step {
        SheetStep::Columns(columns) => view.columns = columns.clone(),
        SheetStep::Sort { column, order } => {
            view.sort = Some((column.clone(), matches!(order, SortOrder::Desc)));
        }
        SheetStep::Filter(text) => view.filter = text.clone(),
        SheetStep::Open(_) | SheetStep::Cell { .. } => return,
    }
    editor.settings.views.insert(table, view);
}

fn model_step(editor: &mut Editor, step: &ModelStep) {
    if let ModelStep::Open(key) = step {
        editor.select(RowSelection::row(ModelDef::TABLE, key));
        editor.open_tab(EditorTab::Model { key: key.clone() });
        return;
    }
    let key = match shell::active_tab(editor) {
        Some(EditorTab::Model { key }) => key,
        _ if editor.selection.table == ModelDef::TABLE && !editor.selection.keys.is_empty() => {
            editor.selection.keys[0].clone()
        }
        _ => return,
    };
    model::ensure_view(editor, &key);
    match step {
        ModelStep::Open(_) => {}
        ModelStep::Busts(face) => {
            let view = editor.views.model.view_mut(&key);
            view.page = ModelPage::Busts;
            if let Some(face) = face {
                view.face = Some(face.clone());
                if let Some(Ok(model)) = editor.read::<ModelDef>(&key)
                    && let Some(found) = busts::faces(&model)
                        .into_iter()
                        .find(|found| found.name == *face)
                {
                    editor.select(RowSelection {
                        table: ModelDef::TABLE.to_owned(),
                        keys: vec![key.clone()],
                        parts: vec![found.part()],
                    });
                }
            }
        }
        ModelStep::Bottom(tab) => editor.panels.bottom = *tab,
        ModelStep::Drop(path) => editor.views.model.drops.push(path.clone()),
    }
}

fn tileset_step(editor: &mut Editor, step: &TilesetStep) {
    if let TilesetStep::Open(key) = step {
        editor.select(RowSelection::row(TilesetDef::TABLE, key));
        editor.open_tab(EditorTab::Tileset { key: key.clone() });
        return;
    }
    let key = match shell::active_tab(editor) {
        Some(EditorTab::Tileset { key }) => key,
        _ if editor.selection.table == TilesetDef::TABLE && !editor.selection.keys.is_empty() => {
            editor.selection.keys[0].clone()
        }
        _ => return,
    };
    tileset::ensure_view(editor, &key);
    match step {
        TilesetStep::Open(_) => {}
        TilesetStep::Mode(mode) => tileset::set_mode(editor, &key, *mode),
        TilesetStep::Brush(brush) => {
            let view = editor.views.tileset.view_mut(&key);
            match brush {
                SceneBrush::Walk(walk) => view.walk = *walk,
                SceneBrush::Surface(surface) => view.surface = Some(surface.clone()),
                SceneBrush::NoSurface => view.surface = None,
            }
        }
        TilesetStep::Select(selection) => {
            let view = editor.views.tileset.view_mut(&key);
            view.selection = Some(SheetRect {
                x: selection.x,
                y: selection.y,
                w: selection.w.max(1),
                h: selection.h.max(1),
            });
            if view.mode == TilesetMode::Animation {
                tileset::select_part(editor, &key, tileset::ANIMATION_PART);
            }
        }
        TilesetStep::Tile(tile) => tileset::select_tile(editor, &key, *tile),
        TilesetStep::Set(set) => editor.views.tileset.view_mut(&key).pick_set(set.clone()),
        TilesetStep::Terrain(index) => editor.views.tileset.view_mut(&key).terrain = *index,
        TilesetStep::Closeup(tile) => {
            let view = editor.views.tileset.view_mut(&key);
            view.closeup = Some(*tile);
            view.tile = Some(*tile);
        }
        TilesetStep::Bottom(tab) => editor.panels.bottom = *tab,
        TilesetStep::Zoom(zoom) => editor.views.tileset.view_mut(&key).zoom = *zoom,
        TilesetStep::Dialog(name) => {
            editor.views.tileset.view_mut(&key).dialog = match name {
                TilesetDialogName::Blueprint => {
                    Some(TileSheetDialog::MakeBlueprint(TileBlueprintDialog {
                        name: format!("{key}_part"),
                        error: None,
                    }))
                }
                TilesetDialogName::Surface => {
                    Some(TileSheetDialog::NewSurface(NewSurfaceDialog::default()))
                }
                TilesetDialogName::Set => Some(TileSheetDialog::NewTerrainSet(
                    NewTerrainSetDialog::default(),
                )),
                TilesetDialogName::None => None,
            };
        }
    }
}

fn world_step(editor: &mut Editor, step: &WorldStep) {
    match step {
        WorldStep::Open => {}
        WorldStep::Lens(lens) => editor.views.world.lenses.push(match lens {
            WorldLens::Row { table, key } => MapLens::Row {
                table: table.clone(),
                key: key.clone(),
            },
            WorldLens::Text(text) => MapLens::Text(text.clone()),
        }),
        WorldStep::Color(field) => editor.views.world.color = field.clone().map(ColorBy::new),
        WorldStep::Line(line) => editor.views.world.quest_line = Some(line.clone()),
        WorldStep::Route(route) => editor.views.world.route = route.clone(),
        WorldStep::Look { area, zoom } => {
            editor.views.world.pending_look = Some(MapLook {
                area: area.clone(),
                zoom: *zoom,
            });
        }
        WorldStep::Select(parts) => world::select(editor, parts.clone()),
        WorldStep::Layer(layer) => {
            let layers = &mut editor.views.world.layers;
            match layer {
                WorldLayer::Residents => layers.residents = true,
                WorldLayer::Populations => layers.populations = true,
                WorldLayer::Givers => layers.givers = true,
            }
        }
        WorldStep::Arrange => arrange::open(editor),
        WorldStep::ArrangeApply => arrange::arrange_everything(editor),
        WorldStep::Region { name, cards } => {
            let Some(map) = OpenWorldMap::of(editor) else {
                return;
            };
            map.change(editor, ContentEdit::new(format!("Add {name}")), |layout| {
                layout.regions.push(Region {
                    name: name.clone(),
                    cards: cards.iter().filter_map(|key| map.atlas.id(key)).collect(),
                    color: None,
                });
            });
        }
        WorldStep::Note { text, x, y } => {
            let Some(map) = OpenWorldMap::of(editor) else {
                return;
            };
            map.change(editor, ContentEdit::new(format!("Add {text}")), |layout| {
                layout.notes.push(Note {
                    text: text.clone(),
                    x: *x,
                    y: *y,
                });
            });
        }
    }
}
