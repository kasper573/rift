use engine::systems::area::tileset::{TileData, TilesetDef};
use strum::VariantArray;
use ui::controls;
use ui::egui;
use ui::egui::Ui;
use ui::form;
use ui::glyph::{self, Mark};
use ui::panel;
use ui::sheet;
use ui::theme;

use super::sheet::TileBrush;
use super::sounds;
use super::terrain::{self, MixCompleteness, TerrainSetTiles, TileCorner};
use super::{
    ANIMATION_PART, NewTerrainSetDialog, OpenTileset, TERRAINS_PART, TileBlueprintDialog,
    TileSheetDialog, TilesetMode, WalkBrush, change_tileset, tile_mut, walk_label,
};
use crate::app::Editor;
use crate::forms::FormCx;
use crate::project::{ContentEdit, RowSelection};
use crate::tabs::shown_path;
use crate::views::area::palette;

pub fn show(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let (Some(key), Some(part)) = (selection.keys.first(), selection.parts.first()) else {
        return;
    };
    super::ensure_view(editor, key);
    let ctx = ui.ctx().clone();
    let Some(tileset) = OpenTileset::read(editor, &ctx, key) else {
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt(("tileset-inspect", key))
        .show(ui, |ui| {
            if let Some(tile) = super::tile_of_part(part) {
                tile_card(editor, ui, &tileset, tile);
                selection_card(editor, ui, &tileset);
                let view = editor.views.tileset.view(key);
                if view.mode == TilesetMode::Surface {
                    let surface = tileset
                        .surface(tile)
                        .map(str::to_owned)
                        .or_else(|| view.surface.clone());
                    if let Some(surface) = surface {
                        ui.add_space(8.0);
                        super::surfaces::card(editor, ui, &surface);
                    }
                }
            } else if part == TERRAINS_PART {
                terrains_cards(editor, ui, &tileset);
            } else if part == ANIMATION_PART {
                super::animation::card(editor, ui, &tileset);
            }
        });
}

pub fn row_body(ui: &mut Ui, cx: &mut FormCx, tileset: &TilesetDef) {
    let image = tileset.image.as_str();
    form::kv_text(ui, "image", shown_path(image));
    let ctx = ui.ctx().clone();
    let tile = tileset.tile;
    let columns = cx
        .state
        .thumbnails
        .image(&ctx, cx.files, image)
        .map(|thumb| thumb.size[0] as u32 / tile.max(1));
    form::kv_text(
        ui,
        "tile",
        match columns {
            Some(columns) => format!("{tile} × {tile} px · {columns} columns, imported"),
            None => format!("{tile} × {tile} px, imported"),
        },
    );
    form::kv_text(ui, "tiles", format!("{} with data", tileset.tiles.len()));
    if !tileset.terrains.is_empty() {
        let names: Vec<&str> = tileset.terrains.keys().map(String::as_str).collect();
        form::kv_text(ui, "terrains", names.join(" · "));
    }
}

fn tile_card(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset, tile: u32) {
    let key = tileset.key.as_str();
    let cell = tileset.sheet.cell(tile);
    ui.label(
        theme::mono(format!("{} › tile {tile}", shown_path(key)))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(4.0);
    let data = tileset.def.data(tile);
    let walk = tileset.def.walk(tile);
    let surface = tileset.surface(tile);
    let mode = editor.views.tileset.view(key).mode;
    let surfaces = sounds::surfaces(editor);
    let mut picked = None;
    panel::card(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::title(ui, &format!("tile {tile} · {}, {}", cell.x, cell.y));
        ui.add_space(4.0);
        if mode == TilesetMode::Surface {
            form::kv(ui, "surface", |ui| {
                controls::combo(("tile-surface", key, tile))
                    .selected_text(theme::mono(surface.unwrap_or("none")))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for option in &surfaces {
                            let on = surface == Some(option.key.as_str());
                            let label = theme::mono(option.name.to_lowercase());
                            if ui.selectable_label(on, label).clicked() {
                                picked = Some(TileBrush::Surface(Some(option.key.clone())));
                            }
                        }
                        if ui
                            .selectable_label(surface.is_none(), theme::mono("none"))
                            .clicked()
                        {
                            picked = Some(TileBrush::Surface(None));
                        }
                    });
            });
            form::kv_text(ui, "walk", walk_label(walk));
        } else {
            form::kv(ui, "walk", |ui| {
                controls::combo(("tile-walk", key, tile))
                    .selected_text(theme::mono(walk_label(walk)))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for brush in WalkBrush::VARIANTS {
                            let on = walk == brush.walk();
                            if ui
                                .selectable_label(on, theme::mono(brush.label()))
                                .clicked()
                            {
                                picked = Some(TileBrush::Walk(*brush));
                            }
                        }
                    });
            });
            form::kv_text(ui, "surface", surface.unwrap_or("none"));
            form::kv_text(ui, "corners", corners_text(data));
            form::kv_text(ui, "animation", animation_text(data));
        }
    });
    if let Some(brush) = picked
        && brush.changes(tileset, tile)
    {
        let edit = ContentEdit::new(format!("Set {} on tile {tile} of {key}", brush.label()));
        change_tileset(editor, key, edit, |def, reader| {
            brush.paint(tile_mut(def, tile), reader);
        });
    }
}

fn selection_card(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let view = editor.views.tileset.view(key);
    let Some(selection) = view.selection else {
        return;
    };
    let brush = TileBrush::of(view);
    sheet::section(ui, &format!("Selection · {}", selection.text()));
    let mut painted = None;
    ui.horizontal_wrapped(|ui| {
        if let Some(brush) = brush {
            let tiles: Vec<u32> = selection
                .tiles(tileset.sheet.columns())
                .filter(|tile| brush.changes(tileset, *tile))
                .collect();
            let response = ui
                .add_enabled(
                    !tiles.is_empty(),
                    egui::Button::new(format!("Paint {} on these tiles", brush.label())),
                )
                .on_disabled_hover_text("Every tile here already has it");
            if response.clicked() {
                painted = Some((brush, tiles));
            }
        }
        if controls::marked(ui, Mark::Grid, "Make blueprint from these tiles…").clicked() {
            let dialog = TileBlueprintDialog {
                name: format!("{key}_{}_{}", selection.x, selection.y),
                error: None,
            };
            editor.views.tileset.view_mut(key).dialog =
                Some(TileSheetDialog::MakeBlueprint(dialog));
        }
        if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
            .on_hover_text("Clear the selection")
            .clicked()
        {
            editor.views.tileset.view_mut(key).selection = None;
        }
    });
    if let Some((brush, tiles)) = painted {
        let edit = ContentEdit::new(format!(
            "Paint {} on {} tiles of {key}",
            brush.label(),
            tiles.len()
        ));
        change_tileset(editor, key, edit, |def, reader| {
            for tile in tiles {
                brush.paint(tile_mut(def, tile), reader);
            }
        });
    }
}

fn terrains_cards(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    ui.label(
        theme::mono(format!("{} › terrains", shown_path(key)))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(4.0);
    let mut picked_set = None;
    let mut removed_set = None;
    let mut toggled_missing = None;
    for (set, terrains) in &tileset.def.terrains {
        let tiles = TerrainSetTiles::of(&tileset.def, set);
        let mixes = MixCompleteness::of(&tileset.def, set);
        let view = editor.views.tileset.view(key);
        let on = view.set.as_ref() == Some(set);
        let missing_shown = view.missing_shown.contains(set);
        let response = panel::card(on)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                panel::header(ui, set, "", |ui| {
                    if ui.button("Remove this set").clicked() {
                        removed_set = Some(set.clone());
                        ui.close();
                    }
                });
                ui.add_space(4.0);
                form::kv(ui, "terrains", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (index, terrain) in terrains.iter().enumerate() {
                            ui.label(theme::mono(terrain).color(palette::terrain_color(index)));
                            if index == 0 {
                                ui.label(theme::faint("base").size(11.0));
                            }
                            if index + 1 < terrains.len() {
                                ui.label(theme::faint("·"));
                            }
                        }
                    });
                });
                let tile_text = match tiles.twins {
                    0 => tiles.tiles.to_string(),
                    twins => format!("{}, {twins} of them twins", tiles.tiles),
                };
                form::kv_text(ui, "tiles", tile_text);
                form::kv(ui, "mixes", |ui| {
                    if mixes.missing.is_empty() {
                        glyph::show(ui, Mark::Check, 14.0, theme::OK);
                        ui.label(format!("all {} beyond its base", mixes.total));
                        return;
                    }
                    ui.label(format!("{} of {} ·", mixes.have, mixes.total));
                    let link = if missing_shown {
                        "hide the missing".to_owned()
                    } else {
                        format!("show the {} missing", mixes.missing.len())
                    };
                    if ui.link(link).clicked() {
                        toggled_missing = Some(set.clone());
                    }
                });
                if missing_shown && !mixes.missing.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for mix in &mixes.missing {
                            ui.label(
                                theme::mono(terrain::mix_code(*mix))
                                    .size(11.5)
                                    .color(theme::MUTED),
                            );
                        }
                    });
                }
            })
            .response
            .interact(egui::Sense::click());
        if response.clicked() {
            picked_set = Some(set.clone());
        }
        ui.add_space(4.0);
    }
    let new_set = ui.button("+ New terrain set").clicked();
    let view = editor.views.tileset.view_mut(key);
    if new_set {
        view.dialog = Some(TileSheetDialog::NewTerrainSet(
            NewTerrainSetDialog::default(),
        ));
    }
    if let Some(set) = picked_set {
        view.pick_set(set);
    }
    if let Some(set) = toggled_missing
        && !view.missing_shown.remove(&set)
    {
        view.missing_shown.insert(set);
    }
    if let Some(set) = removed_set {
        terrain::remove_set(editor, tileset, &set);
    }
    corners_grid(editor, ui, tileset);
}

fn corners_grid(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let view = editor.views.tileset.view(key);
    let (Some(tile), Some(set)) = (view.closeup, view.set.clone()) else {
        return;
    };
    let terrains = tileset.terrains(&set);
    let corners = tileset.corners(tile, &set);
    sheet::section(ui, &format!("Tile {tile} · corners"));
    let mut picked = None;
    egui::Grid::new(("corners", key, tile))
        .spacing(egui::vec2(8.0, 6.0))
        .show(ui, |ui| {
            for corner in TileCorner::VARIANTS {
                ui.label(theme::muted(corner.code()));
                let current = corners.map_or_else(
                    || terrains.first().cloned().unwrap_or_default(),
                    |corners| corners[corner.index()].clone(),
                );
                controls::combo(("corner", key, tile, corner.index()))
                    .selected_text(theme::mono(&current))
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for (index, terrain) in terrains.iter().enumerate() {
                            if ui
                                .selectable_label(*terrain == current, theme::mono(terrain))
                                .clicked()
                            {
                                picked = Some((*corner, index));
                            }
                        }
                    });
                if !corner.west() {
                    ui.end_row();
                }
            }
        });
    if let Some((corner, terrain)) = picked {
        terrain::set_corner(editor, tileset, &set, tile, corner, terrain);
    }
}

fn corners_text(data: Option<&TileData>) -> String {
    let Some(data) = data.filter(|data| !data.corners.is_empty()) else {
        return "none".to_owned();
    };
    data.corners
        .iter()
        .map(|(set, corners)| format!("{set}: {}", corners.join(" ")))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn animation_text(data: Option<&TileData>) -> String {
    let Some(data) = data.filter(|data| !data.animation.is_empty()) else {
        return "none".to_owned();
    };
    let ms: u32 = data.animation.iter().map(|frame| frame.ms).sum();
    format!("{} frames · {ms} ms", data.animation.len())
}
