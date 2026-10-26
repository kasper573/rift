use std::path::PathBuf;

use engine::core::content::ContentFile;
use engine::systems::actor::model::ModelDef;
use engine::systems::actor::skeleton::{ClipName, SkeletonDef};
use engine::systems::area::AreaDef;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::prop::PropDef;
use engine::systems::area::world_map::{Note, Region};
use engine::systems::class::ClassDef;
use engine::systems::item::ItemDef;
use engine::systems::script::ScriptDef;
use serde::Deserialize;
use strum::VariantNames;

use crate::app::{BottomTab, Clipboard, Editor, ModalDialog, SceneClip};
use crate::project::{ContentEdit, RowSelection};
use crate::tabs::EditorTab;
use crate::views::area::resize::ResizeDialog;
use crate::views::area::{self as area_view, blueprint};
use crate::views::character::CharacterSection;
use crate::views::class;
use crate::views::item::{self, ItemPage};
use crate::views::model::{self, ModelPage, busts};
use crate::views::script::{PendingScriptAction, ScriptRow};
use crate::views::skeleton::{self, SkeletonPage};
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
    Prop(String),
    /// Takes a tool in the open area view, with a prop or blueprint in hand when one is named.
    Tool {
        tool: area_view::AreaTool,
        #[serde(default)]
        prop: Option<String>,
        #[serde(default)]
        blueprint: Option<String>,
        #[serde(default)]
        terrain: Option<String>,
    },
    Overlay(area_view::AreaOverlay),
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
    Skeleton(SkeletonStep),
    /// Opens the item at `key` on `page`.
    Item {
        key: String,
        page: ItemPage,
    },
    Sheet(SheetStep),
    World(WorldStep),
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
    Page(ModelPage),
    Busts(Option<String>),
    Bottom(BottomTab),
    Drop(PathBuf),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkeletonStep {
    Open(String),
    Page(SkeletonPage),
    /// Lights the joint of this name on the joints page.
    Joint(String),
    /// Plays the library clip of this name on the stage, none standing the rig at rest.
    Clip(Option<String>),
    /// Shows the rig as the model at this path.
    Wearer(String),
    /// Hangs the item at this path from the stage's model.
    Trying(String),
    /// Chooses the hand at this place in the list on the hands page.
    Hand(usize),
    /// Chooses the layer at this place in the list on the layers page.
    Layer(usize),
    /// Opens the picker of the sets page's cell for this row in the stance at this path, base when
    /// none.
    Cell {
        row: skeleton::sets::SetRow,
        #[serde(default)]
        stance: Option<String>,
    },
    Drop(PathBuf),
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

/// The area or blueprint the scene opened last, whose tab may not be placed in the dock yet.
fn last_opened_bundle(editor: &Editor) -> Option<String> {
    editor
        .open_tabs
        .iter()
        .rev()
        .find_map(|(tab, _)| match tab {
            EditorTab::Area { key } | EditorTab::Blueprint { key } => Some(key.clone()),
            _ => None,
        })
        .or_else(|| match shell::active_tab(editor)? {
            EditorTab::Area { key } | EditorTab::Blueprint { key } => Some(key),
            _ => None,
        })
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
        SceneStep::Prop(key) => {
            editor.select(RowSelection::row(PropDef::TABLE, key));
            editor.open_tab(EditorTab::Prop { key: key.clone() });
        }
        SceneStep::Tool {
            tool,
            prop,
            blueprint,
            terrain,
        } => {
            if let Some(bundle) = last_opened_bundle(editor) {
                let view = editor.views.area.ensure_view(&bundle);
                view.set_tool(*tool);
                if let Some(terrain) = terrain {
                    view.paint_terrain = Some(terrain.clone());
                }
                view.placeable = match (prop, blueprint) {
                    (Some(prop), _) => Some(area_view::Placeable::Prop(prop.clone())),
                    (_, Some(blueprint)) => {
                        Some(area_view::Placeable::Blueprint(blueprint.clone()))
                    }
                    _ => None,
                };
            }
        }
        SceneStep::Overlay(overlay) => {
            if let Some(bundle) = last_opened_bundle(editor) {
                editor.views.area.ensure_view(&bundle).toggle(*overlay);
            }
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
            editor.open_tab(EditorTab::Area { key: key.clone() });
            if let Some(Ok(area)) = editor.read::<AreaDef>(key) {
                editor
                    .dialogs
                    .push(ModalDialog::Resize(ResizeDialog::for_bundle(
                        key, area.size,
                    )));
            }
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
        SceneStep::Skeleton(step) => skeleton_step(editor, step),
        SceneStep::Item { key, page } => {
            editor.open_row(ItemDef::TABLE, key);
            item::ensure_view(editor, key);
            editor.views.item.view_mut(key).page = *page;
        }
        SceneStep::Sheet(step) => sheet_step(editor, step),
        SceneStep::World(step) => {
            editor.open_tab(EditorTab::World);
            world_step(editor, step);
        }
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
        ModelStep::Page(page) => editor.views.model.view_mut(&key).page = *page,
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

fn skeleton_step(editor: &mut Editor, step: &SkeletonStep) {
    if let SkeletonStep::Open(key) = step {
        editor.open_row(SkeletonDef::TABLE, key);
        skeleton::ensure_view(editor, key);
        return;
    }
    let key = match shell::active_tab(editor) {
        Some(EditorTab::Row { table, key }) if table == SkeletonDef::TABLE => key,
        _ if editor.selection.table == SkeletonDef::TABLE && !editor.selection.keys.is_empty() => {
            editor.selection.keys[0].clone()
        }
        _ => return,
    };
    skeleton::ensure_view(editor, &key);
    let view = editor.views.skeleton.view_mut(&key);
    match step {
        SkeletonStep::Open(_) => {}
        SkeletonStep::Page(page) => view.page = *page,
        SkeletonStep::Joint(joint) => view.joint = Some(joint.clone()),
        SkeletonStep::Clip(clip) => view.clip = clip.as_deref().map(ClipName::new),
        SkeletonStep::Wearer(model) => view.wearer = Some(model.clone()),
        SkeletonStep::Trying(item) => view.trying = Some(item.clone()),
        SkeletonStep::Hand(hand) => view.hand = *hand,
        SkeletonStep::Layer(layer) => view.layers.layer = *layer,
        SkeletonStep::Cell { row, stance } => {
            view.page = SkeletonPage::Sets;
            view.sets.open = Some((row.clone(), stance.clone()));
        }
        SkeletonStep::Drop(path) => editor.views.skeleton.drops.push(path.clone()),
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
