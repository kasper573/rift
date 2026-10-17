use egui::{RichText, Ui};
use engine::systems::area::tileset::TileData;
use serde_json::Value;
use strum::VariantArray;

use super::sheet::TileBrush;
use super::terrain::{self, MixCompleteness, TerrainSetTiles, TileCorner};
use super::{
    ANIMATION_PART, NewTerrainSetDialog, OpenTileset, TERRAINS_PART, TileBlueprintDialog,
    TileSheetDialog, TilesetMode, WalkBrush, change_tileset, tile_mut, walk_label,
};
use crate::app::Editor;
use crate::forms::{FormChange, FormCx};
use crate::picker;
use crate::project::{ContentEdit, RowSelection};
use crate::tabs::EditorTab;
use crate::ui::assets::AssetKind;
use crate::ui::{inspector, theme, widgets};
use crate::views::area::palette;
use crate::views::model::sounds;

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
                selection_card(editor, ui, key);
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
                } else {
                    ui.add_space(8.0);
                    inspector::show_row(editor, ui, super::TABLE, key);
                }
                used_by(editor, ui, key);
            } else if part == TERRAINS_PART {
                terrains_cards(editor, ui, &tileset);
            } else if part == ANIMATION_PART {
                super::animation::card(editor, ui, &tileset);
            }
        });
}

pub fn row_body(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let mut changed = FormChange::default();
    widgets::kv(ui, "image", |ui| {
        let mut file = value["image"].as_str().unwrap_or_default().to_owned();
        if picker::asset_of(ui, cx, &[AssetKind::Image], &mut file, "/image") {
            value["image"] = Value::String(file);
            changed = FormChange::at("/image");
        }
    });
    let tile = value["tile"].as_u64().unwrap_or(16);
    let ctx = ui.ctx().clone();
    let columns = value["image"]
        .as_str()
        .and_then(|image| cx.state.thumbnails.image(&ctx, cx.files, image))
        .map(|thumb| thumb.size[0] as u64 / tile.max(1));
    widgets::kv_text(
        ui,
        "tile",
        match columns {
            Some(columns) => format!("{tile} × {tile} px · {columns} columns, imported"),
            None => format!("{tile} × {tile} px, imported"),
        },
    );
    let tiles = value["tiles"].as_object().map_or(0, |tiles| tiles.len());
    widgets::kv_text(ui, "tiles", format!("{tiles} with data"));
    if let Some(sets) = value["terrains"]
        .as_object()
        .filter(|sets| !sets.is_empty())
    {
        let names: Vec<&str> = sets.keys().map(String::as_str).collect();
        widgets::kv_text(ui, "terrains", names.join(" · "));
    }
    changed
}

fn tile_card(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset, tile: u32) {
    let key = tileset.key.as_str();
    let cell = tileset.sheet.cell(tile);
    ui.label(
        theme::mono(format!("tileset/{key} › tile {tile}"))
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
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, &format!("tile {tile} · {}, {}", cell.x, cell.y));
        ui.add_space(4.0);
        if mode == TilesetMode::Surface {
            widgets::kv(ui, "surface", |ui| {
                widgets::combo_box(("tile-surface", key, tile))
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
            widgets::kv_text(ui, "walk", walk_label(walk));
        } else {
            widgets::kv(ui, "walk", |ui| {
                widgets::combo_box(("tile-walk", key, tile))
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
            widgets::kv_text(ui, "surface", surface.unwrap_or("none"));
            widgets::kv_text(ui, "corners", corners_text(data));
            widgets::kv_text(ui, "animation", animation_text(data));
        }
    });
    if let Some(brush) = picked
        && brush.changes(tileset, tile)
    {
        let content = tileset.content.clone();
        let edit = ContentEdit::new(format!("Set {} on tile {tile} of {key}", brush.label()));
        change_tileset(editor, key, edit, |def| {
            brush.paint(tile_mut(def, tile), &content);
        });
    }
}

fn selection_card(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(selection) = editor.views.tileset.view(key).selection else {
        return;
    };
    widgets::section(ui, &format!("Selection · {}", selection.text()));
    ui.horizontal(|ui| {
        if ui.button("▣ Make blueprint from these tiles…").clicked() {
            let dialog = TileBlueprintDialog {
                name: format!("{key}_{}_{}", selection.x, selection.y),
                error: None,
            };
            editor.views.tileset.view_mut(key).dialog =
                Some(TileSheetDialog::MakeBlueprint(dialog));
        }
        if ui
            .small_button("✕")
            .on_hover_text("Clear the selection")
            .clicked()
        {
            editor.views.tileset.view_mut(key).selection = None;
        }
    });
}

fn used_by(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let areas = editor.views.tileset.areas_using(key);
    widgets::section_count(ui, "Used by", format!("{} areas", areas.len()));
    if areas.is_empty() {
        widgets::hint(ui, "no area's palette names this tileset");
    }
    let mut open = None;
    for area in &areas {
        ui.horizontal(|ui| {
            widgets::badge(ui, "area");
            if ui.link(theme::mono(area)).clicked() {
                open = Some(area.clone());
            }
        });
    }
    if let Some(area) = open {
        editor.select(RowSelection::row("area", &area));
        editor.open_tab(EditorTab::Area { key: area });
    }
}

fn terrains_cards(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    ui.label(
        theme::mono(format!("tileset/{key} › terrains"))
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
        let response = widgets::card_frame(on)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::card_header(ui, set, "", |ui| {
                    if ui.button("Remove this set").clicked() {
                        removed_set = Some(set.clone());
                        ui.close();
                    }
                });
                ui.add_space(4.0);
                widgets::kv(ui, "terrains", |ui| {
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
                widgets::kv_text(ui, "tiles", tile_text);
                widgets::kv(ui, "mixes", |ui| {
                    if mixes.missing.is_empty() {
                        ui.label(RichText::new("✓").color(theme::OK));
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
                        for code in &mixes.missing {
                            ui.label(theme::mono(code).size(11.5).color(theme::MUTED));
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
    widgets::section(ui, &format!("Tile {tile} · corners"));
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
                widgets::combo_box(("corner", key, tile, corner.index()))
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
