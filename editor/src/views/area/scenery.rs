use engine::core::math::Pos;
use engine::core::units::{Cell, Meters, TilePos};
use engine::systems::area::object::{ObjectDef, ObjectShape};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{self, AreaDef, AreaModule, Scenery, SceneryTile};
use engine::systems::script::Place;
use ui::controls;
use ui::egui::Ui;
use ui::form;
use ui::glyph::Mark;
use ui::input;

use super::grid::GridDraft;
use super::inspect::Followups;
use super::placement::{self, warp_destinations};
use super::stamp::object_names;
use super::{AreaFrame, AreaSnapshot, nodes};
use crate::forms::FormCx;
use crate::project::Project;
use crate::tabs::file_title;
use crate::{app::Editor, rows};
use ui::input::InputField;

pub struct PlacedTile {
    pub column: usize,
    pub row: usize,
    pub offset: (usize, usize),
    pub tile: SceneryTile,
}

pub fn tiles_of(scenery: &Scenery) -> Vec<PlacedTile> {
    let flip = scenery.flip;
    let height = scenery.height();
    scenery
        .tiles
        .iter()
        .enumerate()
        .flat_map(|(row, cells)| {
            let width = cells.len();
            cells.iter().enumerate().filter_map(move |(column, tile)| {
                let tile = (*tile)?;
                Some(PlacedTile {
                    column,
                    row,
                    offset: (
                        if flip.horizontal() {
                            width - 1 - column
                        } else {
                            column
                        },
                        if flip.vertical() {
                            height - 1 - row
                        } else {
                            row
                        },
                    ),
                    tile: SceneryTile {
                        tile: tile.tile,
                        flip: tile.flip.compose(flip),
                    },
                })
            })
        })
        .collect()
}

pub fn tile_count(scenery: &Scenery) -> usize {
    tiles_of(scenery).len()
}

pub fn split_scenery(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let Some(AreaModule::Scenery(scenery)) = frame.module(node) else {
        return;
    };
    let rect = scenery.bounds();
    let Some(tileset) = frame.reader.path_of(scenery.tileset).map(str::to_owned) else {
        return;
    };
    let Some(mut draft) = GridDraft::own(editor, &frame.bundle) else {
        return;
    };
    let last = editor
        .views
        .area
        .views
        .get(&super::view_key(&frame.bundle))
        .and_then(|view| view.last_layer.clone());
    let Some(layer) = last
        .and_then(|name| draft.layer_index(&name))
        .or(draft.ground.len().checked_sub(1))
    else {
        editor.toast("There is no ground layer to split the scenery into");
        return;
    };
    let mut kept = scenery.tiles.clone();
    let mut placed = 0;
    for tile in tiles_of(scenery) {
        let cell = Pos::<Meters>::new(
            rect.min().x + tile.offset.0 as f32 + 0.5,
            rect.min().y + tile.offset.1 as f32 + 0.5,
        )
        .cell();
        if draft.get(layer, cell).is_some() || cell.index(draft.size).is_none() {
            continue;
        }
        let entry = draft.intern(&tileset, tile.tile.tile, tile.tile.flip);
        draft.set(layer, cell, Some(entry));
        kept[tile.row][tile.column] = None;
        placed += 1;
    }
    if placed == 0 {
        editor.toast(format!(
            "No free cell under the scenery in {}",
            draft.ground[layer].name
        ));
        return;
    }
    let all_gone = kept.iter().flatten().all(Option::is_none);
    let applied = nodes::change_blueprint(editor, &frame.bundle, |blueprint| {
        let scene = &mut blueprint.scene.nodes;
        if all_gone {
            nodes::SceneTree::new(scene)
                .remove(&[node.to_owned()])
                .first()?;
        } else if let AreaModule::Scenery(scenery) =
            &mut nodes::module_node_mut(scene, node)?.module
        {
            scenery.tiles = kept;
        }
        draft.apply_to(blueprint);
        Some(format!("Split scenery into {placed} tiles"))
    });
    if applied && all_gone {
        editor.selection.parts.clear();
    }
}

pub fn return_door(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let area_key = &frame.bundle;
    let Some(AreaModule::Object(object)) = frame.module(node) else {
        return;
    };
    let Some((target_area, target_at)) = warp_destinations(object, frame.reader.as_ref())
        .into_iter()
        .next()
    else {
        editor.toast("The object has no warp to return from");
        return;
    };
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return;
    };
    let Some(here_id) = content.by_path::<AreaDef>(area_key) else {
        return;
    };
    let Some(there_id) = content.by_path::<AreaDef>(&target_area) else {
        return;
    };
    let here = area::load(&content, here_id);
    let there = area::load(&content, there_id);
    let door_bounds = object.shape.bounds();
    let back_to = here
        .grid
        .nodes()
        .iter()
        .filter(|cell| !door_bounds.contains(**cell))
        .min_by(|a, b| {
            a.distance(door_bounds.center())
                .0
                .total_cmp(&b.distance(door_bounds.center()).0)
        })
        .copied();
    let Some(back_to) = back_to else {
        editor.toast("No walkable cell outside the door to come back to");
        return;
    };
    let arrival = target_at.cell();
    let Some(beside_arrival) = there
        .grid
        .nearest_walkable_where(target_at, |cell| cell != arrival)
    else {
        editor.toast("The destination has no walkable cell beside the arrival for a door back");
        return;
    };
    let mut names = object_names(area::placed(&content, there_id));
    let Some(here_in_files) = frame.reader.id_of::<AreaDef>(area_key) else {
        return;
    };
    let name = rows::free_name(
        &format!("{}_back", object.name.as_deref().unwrap_or("door")),
        &names,
    );
    names.insert(name.clone());
    let cell = beside_arrival.cell();
    let mut door = ObjectDef {
        name: Some(name),
        title: None,
        shape: ObjectShape::Point(cell.center()),
        look: None,
        shown: None,
        scripts: object.scripts.clone(),
    };
    placement::redirect_places(
        &mut door,
        Place {
            area: here_in_files,
            at: back_to,
        },
    );
    let door = SceneNode::Module(ModuleNode::new(AreaModule::Object(door)));
    let applied = nodes::change_scene(editor, &target_area, |scene| {
        nodes::SceneTree::new(scene).append("", vec![door]);
        Some(format!(
            "Return door from {} to {}",
            file_title(&target_area),
            file_title(area_key)
        ))
    });
    if applied {
        editor.toast(format!(
            "Added a door back in {target_area} at {}, {}",
            cell.x, cell.y
        ));
    }
}

pub(super) fn scenery_rows(ui: &mut Ui, scenery: &mut Scenery, path: &str) -> bool {
    let (width, height) = (scenery.width(), scenery.height());
    let count = tile_count(scenery);
    form::kv_text(ui, "size", format!("{width} × {height} tiles · {count}"));
    let mut changed = false;
    form::kv(ui, "sort", |ui| {
        let current = scenery.sort;
        let id = ui.id().with((path, "sort"));
        let shown = current
            .map(|number| number.to_string())
            .unwrap_or_else(|| "flat".to_owned());
        let unit = format!("tiles from the top · of {height}");
        let typed = input::drafted(ui, id, &shown, |ui, draft| {
            InputField::new(draft).unit(&unit).show(ui)
        });
        if let Some(typed) = typed {
            let parsed = typed.trim().parse::<f32>().ok();
            if parsed != current {
                scenery.sort = parsed;
                changed = true;
            }
        }
    });
    let blocks = scenery.blocks.len();
    form::kv_text(
        ui,
        "blocks",
        if blocks == 0 {
            "none of its own".to_owned()
        } else {
            format!("{blocks} cells of its own")
        },
    );
    changed
}

pub(super) fn scenery_extras(
    ui: &mut Ui,
    cx: &mut FormCx,
    scenery: &Scenery,
    out: &mut Followups,
    node: &str,
) {
    let under = cx
        .row_key
        .as_deref()
        .and_then(|key| blocked_under(cx.project, key, scenery))
        .unwrap_or(0);
    ui.add_space(4.0);
    if under > 0 {
        form::hint(ui, &format!("{under} cells under it are blocked"));
        if ui
            .button(format!("Make the {under} blocked cells its own"))
            .clicked()
        {
            out.actions
                .push(super::AreaAction::AdoptBlocks(node.to_owned()));
        }
    } else {
        form::hint(ui, "No cells under it are blocked");
    }
    ui.horizontal(|ui| {
        if controls::marked(ui, Mark::Grid, "Make blueprint…").clicked() {
            out.actions.push(super::AreaAction::MakeBlueprint);
        }
        if ui.button("Split into tiles").clicked() {
            out.actions.push(super::AreaAction::Split(node.to_owned()));
        }
    });
}

pub(super) fn blocked_under(project: &Project, bundle: &str, scenery: &Scenery) -> Option<usize> {
    let draft = GridDraft::own_in(project, bundle)?;
    Some(
        scenery
            .cells()
            .filter(|(_, cell)| draft.walk_at(*cell) == Some(false))
            .count(),
    )
}

pub fn adopt_blocks(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let Some(AreaModule::Scenery(scenery)) = frame.module(node) else {
        return;
    };
    let Some(mut draft) = GridDraft::own(editor, &frame.bundle) else {
        return;
    };
    let mut adopted = scenery.blocks.clone();
    let mut count = 0;
    for (tile, cell) in scenery.cells() {
        if draft.walk_at(cell) == Some(false) {
            draft.set_walk(cell, None);
            if !adopted.contains(&tile) {
                adopted.push(tile);
            }
            count += 1;
        }
    }
    if count == 0 {
        editor.toast("Nothing under the scenery is blocked on the walk grid");
        return;
    }
    nodes::change_blueprint(editor, &frame.bundle, |blueprint| {
        let AreaModule::Scenery(scenery) =
            &mut nodes::module_node_mut(&mut blueprint.scene.nodes, node)?.module
        else {
            return None;
        };
        scenery.blocks = adopted;
        draft.apply_to(blueprint);
        Some(format!("Give {count} blocked cells to the scenery"))
    });
}

pub fn selected_scenery(editor: &Editor, frame: &AreaFrame) -> Option<String> {
    let parts = super::selected_parts(editor, &frame.bundle);
    if parts.len() != 1 {
        return None;
    }
    let node = parts.into_iter().next()?;
    matches!(frame.module(&node)?, AreaModule::Scenery(_)).then_some(node)
}
