use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Color32, Layout, Sense, Stroke as EStroke, Ui, Vec2};
use engine::core::content::Content;
use engine::systems::area::tileset::TilesetDef;
use engine::systems::area::{self, AreaId};
use strum::VariantArray;

use super::brush::{TerrainPick, TilePick};
use crate::app::Editor;
use crate::ui::{theme, widgets};
use crate::views::tileset::terrain::MixCompleteness;
use crate::views::tileset::terrain::TileCorner;
use crate::views::tileset::{SheetLayout, SheetRect, TilesetSheet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteLens {
    #[default]
    Tiles,
    Terrains,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str, content: &Content, area_id: AreaId) {
    let tile_px = area::load(content, area_id).tile;
    let tilesets: Vec<(String, &TilesetDef)> = content
        .table::<TilesetDef>()
        .iter()
        .filter(|(_, def)| def.tile == tile_px)
        .map(|(id, def)| (id.key(content).to_owned(), def))
        .collect();
    let used = area::load(content, area_id).grids.tilesets();
    let view = editor.views.area.view(view_key);
    let stored = view.palette_lens;
    let lens = if view.tool == super::AreaTool::Terrain {
        PaletteLens::Terrains
    } else {
        stored
    };
    let mut picked = stored;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (value, label) in [
            (PaletteLens::Tiles, "tiles"),
            (PaletteLens::Terrains, "terrains"),
        ] {
            let on = lens == value;
            if widgets::toggle_chip(ui, label, on).clicked() {
                picked = value;
            }
        }
        ui.add_space(12.0);
        match lens {
            PaletteLens::Tiles => tileset_chips(editor, ui, view_key, &tilesets, &used),
            PaletteLens::Terrains => terrain_chips(editor, ui, view_key, &tilesets, &used),
        }
    });
    editor.views.area.view_mut(view_key).palette_lens = picked;
    ui.add_space(6.0);
    match lens {
        PaletteLens::Tiles => tile_sheet(editor, ui, view_key, content, &tilesets),
        PaletteLens::Terrains => terrain_sheet(editor, ui, view_key, &tilesets),
    }
}

fn tileset_chips(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    tilesets: &[(String, &TilesetDef)],
    used: &BTreeSet<String>,
) {
    let view = editor.views.area.view_mut(view_key);
    let mut own: Vec<&String> = tilesets
        .iter()
        .map(|(k, _)| k)
        .filter(|k| used.contains(*k))
        .collect();
    own.sort();
    let mut others: Vec<(String, String)> = tilesets
        .iter()
        .map(|(k, _)| k)
        .filter(|k| !used.contains(*k))
        .map(|k| (k.clone(), k.clone()))
        .collect();
    others.sort();
    if view.pick.tileset.is_none() {
        view.pick.tileset = own
            .first()
            .map(|k| (*k).clone())
            .or_else(|| others.first().map(|(k, _)| k.clone()));
    }
    let current = view.pick.tileset.clone();
    let mut picked: Option<String> = None;
    for key in &own {
        let on = current.as_deref() == Some(key.as_str());
        if widgets::toggle_chip(ui, key, on).clicked() {
            picked = Some((*key).clone());
        }
    }
    if let Some(key) = current.as_ref().filter(|k| !own.contains(k)) {
        widgets::toggle_chip(ui, key, true);
    }
    let id = ui.make_persistent_id((view_key, "other tilesets"));
    if let Some(key) = more_popup(ui, id, &mut view.tileset_search, "Search tilesets", &others) {
        picked = Some(key);
    }
    if let Some(key) = picked {
        view.pick = TilePick {
            tileset: Some(key),
            ..TilePick::default()
        };
    }
}

fn more_popup<T: Clone>(
    ui: &mut Ui,
    id: egui::Id,
    search: &mut String,
    placeholder: &str,
    others: &[(String, T)],
) -> Option<T> {
    if others.is_empty() {
        return None;
    }
    let mut picked = None;
    let response = widgets::chip(ui, &format!("+ {}", others.len()));
    egui::Popup::from_toggle_button_response(&response)
        .id(id)
        .width(280.0)
        .show(|ui| {
            ui.set_min_width(240.0);
            widgets::search_box(ui, search, placeholder, None);
            let needle = search.to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for (label, item) in others.iter().filter(|(label, _)| {
                        needle.is_empty() || label.to_lowercase().contains(&needle)
                    }) {
                        if ui.selectable_label(false, theme::mono(label)).clicked() {
                            picked = Some(item.clone());
                        }
                    }
                });
        });
    picked
}

fn tile_sheet(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    content: &Content,
    tilesets: &[(String, &TilesetDef)],
) {
    let Some(tileset_key) = editor.views.area.view(view_key).pick.tileset.clone() else {
        widgets::hint(ui, "No tileset of this tile size.");
        return;
    };
    let Some(def) = tilesets
        .iter()
        .find(|(k, _)| *k == tileset_key)
        .map(|(_, d)| *d)
    else {
        return;
    };
    let random = editor.settings.brush.random;
    let Some(sheet) = sheet_of(editor, ui, def) else {
        return;
    };
    let shown = sheet.fit(ui.available_width(), 12.0);
    let columns = sheet.columns();
    egui::ScrollArea::vertical()
        .id_salt((view_key, "tiles"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(sheet.size(shown), Sense::click_and_drag());
            let layout = SheetLayout { rect, shown };
            let painter = ui.painter_at(rect);
            sheet.draw_sheet(&painter, rect);
            layout.draw_lines(
                &painter,
                &sheet,
                EStroke::new(1.0, Color32::from_black_alpha(70)),
            );
            let view = editor.views.area.view_mut(view_key);
            if let Some(p) = response.interact_pointer_pos() {
                let cell = layout.clamped_cell(&sheet, p);
                let tile = cell.y * columns + cell.x;
                let ctrl = ui.input(|i| i.modifiers.ctrl);
                if response.drag_started() || response.clicked() {
                    if random || ctrl {
                        if let Some(at) = view.pick.random.iter().position(|t| *t == tile) {
                            view.pick.random.remove(at);
                        } else {
                            view.pick.random.push(tile);
                        }
                        view.pick.block.clear();
                    } else {
                        view.pick.anchor = Some(cell);
                        view.pick.block = vec![vec![tile]];
                        view.pick.random.clear();
                    }
                } else if response.dragged()
                    && !random
                    && !ctrl
                    && let Some(anchor) = view.pick.anchor
                {
                    view.pick.block = SheetRect::spanning(anchor, cell).rows(columns);
                }
            }
            let highlight = |tile: u32| layout.tile_rect(&sheet, tile);
            let block: Vec<u32> = view.pick.block.iter().flatten().copied().collect();
            if let (Some(first), Some(last)) = (block.first(), block.last()) {
                painter.rect_stroke(
                    highlight(*first).union(highlight(*last)),
                    0.0,
                    EStroke::new(2.0, theme::ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            for tile in &view.pick.random {
                painter.rect_stroke(
                    highlight(*tile),
                    0.0,
                    EStroke::new(2.0, theme::ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(p) = response.hover_pos() {
                let cell = layout.clamped_cell(&sheet, p);
                let tile = cell.y * columns + cell.x;
                painter.rect_stroke(
                    highlight(tile),
                    0.0,
                    EStroke::new(1.0, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                let mut info = format!("{tileset_key} #{tile}");
                if let Some(data) = def.data(tile) {
                    if let Some(walk) = data.walk {
                        info.push_str(&format!(" · {}", <&str>::from(walk)));
                    }
                    if let Some(surface) = data.surface {
                        info.push_str(&format!(" · {}", surface.key(content)));
                    }
                    if !data.animation.is_empty() {
                        info.push_str(&format!(" · {} frames", data.animation.len()));
                    }
                }
                editor.status.left.push(info);
            }
        });
}

fn sheet_of(editor: &mut Editor, ui: &mut Ui, def: &TilesetDef) -> Option<TilesetSheet> {
    let sheet = TilesetSheet::of(editor, ui.ctx(), def);
    if sheet.thumb.is_none() {
        ui.label(theme::muted(format!("{} is missing", def.image.as_str())));
        return None;
    }
    Some(sheet)
}

fn terrain_sets(
    tilesets: &[(String, &TilesetDef)],
    used: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut sets: Vec<(String, String)> = tilesets
        .iter()
        .flat_map(|(key, def)| {
            def.terrains
                .keys()
                .map(move |set| (key.clone(), set.clone()))
        })
        .collect();
    sets.sort_by_key(|(tileset, set)| (!used.contains(tileset), tileset.clone(), set.clone()));
    sets
}

fn terrain_chips(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    tilesets: &[(String, &TilesetDef)],
    used: &BTreeSet<String>,
) {
    let sets = terrain_sets(tilesets, used);
    let view = editor.views.area.view_mut(view_key);
    if view.terrain.set.is_none()
        && let Some((tileset, set)) = sets.first()
    {
        view.terrain = TerrainPick {
            tileset: Some(tileset.clone()),
            set: Some(set.clone()),
            terrain: 0,
        };
    }
    let mut picked: Option<(String, String)> = None;
    let label = |tileset: &str, set: &str| format!("{set} · {tileset}");
    let own: Vec<&(String, String)> = sets
        .iter()
        .filter(|(tileset, _)| used.contains(tileset))
        .collect();
    let others: Vec<(String, (String, String))> = sets
        .iter()
        .filter(|(tileset, _)| !used.contains(tileset))
        .map(|pick| (label(&pick.0, &pick.1), pick.clone()))
        .collect();
    for (tileset, set) in &own {
        let on = view.terrain.tileset.as_deref() == Some(tileset)
            && view.terrain.set.as_deref() == Some(set);
        if widgets::toggle_chip(ui, &label(tileset, set), on).clicked() {
            picked = Some((tileset.clone(), set.clone()));
        }
    }
    if let (Some(tileset), Some(set)) = (&view.terrain.tileset, &view.terrain.set)
        && !own.iter().any(|(t, s)| t == tileset && s == set)
    {
        widgets::toggle_chip(ui, &label(tileset, set), true);
    }
    let id = ui.make_persistent_id((view_key, "other terrain sets"));
    if let Some(pick) = more_popup(
        ui,
        id,
        &mut view.tileset_search,
        "Search terrain sets",
        &others,
    ) {
        picked = Some(pick);
    }
    if let Some((tileset, set)) = picked {
        view.terrain = TerrainPick {
            tileset: Some(tileset),
            set: Some(set),
            terrain: 0,
        };
    }
}

fn terrain_sheet(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    tilesets: &[(String, &TilesetDef)],
) {
    let pick = editor.views.area.view(view_key).terrain.clone();
    let (Some(tileset_key), Some(set)) = (pick.tileset.clone(), pick.set.clone()) else {
        widgets::hint(ui, "No tileset of this tile size names terrains.");
        return;
    };
    let Some(def) = tilesets
        .iter()
        .find(|(k, _)| *k == tileset_key)
        .map(|(_, d)| *d)
    else {
        return;
    };
    let Some(terrains) = def.terrains.get(&set).cloned() else {
        return;
    };
    let Some(sheet) = sheet_of(editor, ui, def) else {
        return;
    };
    let legend_width = 150.0;
    let shown = sheet.fit(ui.available_width() - legend_width - 12.0, 8.0);
    let columns = sheet.columns();
    let size = sheet.size(shown);
    let corners: BTreeMap<u32, [String; 4]> = def
        .tiles
        .iter()
        .filter_map(|(tile, data)| data.corners.get(&set).map(|c| (*tile, c.clone())))
        .collect();
    ui.horizontal_top(|ui| {
        egui::ScrollArea::vertical()
            .id_salt((view_key, "terrain sheet"))
            .auto_shrink([false, false])
            .max_width(size.x)
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
                let layout = SheetLayout { rect, shown };
                let painter = ui.painter_at(rect);
                sheet.draw_sheet(&painter, rect);
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(90));
                for (tile, mix) in &corners {
                    let cell = layout.tile_rect(&sheet, *tile);
                    for corner in TileCorner::VARIANTS {
                        let terrain = &mix[corner.index()];
                        let Some(index) = terrains.iter().position(|t| t == terrain) else {
                            continue;
                        };
                        let color = theme::alpha(terrain_color(index), 110);
                        painter.rect_filled(corner.quarter(cell).shrink(0.5), 0.0, color);
                    }
                    painter.rect_stroke(
                        cell,
                        0.0,
                        EStroke::new(1.0, Color32::from_white_alpha(60)),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(p) = response.hover_pos() {
                    let cell = layout.clamped_cell(&sheet, p);
                    let tile = cell.y * columns + cell.x;
                    let status = match corners.get(&tile) {
                        Some(mix) => format!("{tileset_key} #{tile} · {}", mix.join(" ")),
                        None => format!("{tileset_key} #{tile} · not in {set}"),
                    };
                    editor.status.left.push(status);
                }
            });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(legend_width);
            let view = editor.views.area.view_mut(view_key);
            for (index, terrain) in terrains.iter().enumerate() {
                let on = view.terrain.terrain == index;
                let response = ui
                    .horizontal(|ui| {
                        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                        ui.painter().rect_filled(swatch, 2.0, terrain_color(index));
                        if on {
                            ui.painter().rect_stroke(
                                swatch,
                                2.0,
                                EStroke::new(1.5, Color32::WHITE),
                                egui::StrokeKind::Outside,
                            );
                        }
                        ui.label(if on {
                            theme::strong(terrain)
                        } else {
                            theme::muted(terrain)
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if index < 9 {
                                widgets::key_hint(ui, &(index + 1).to_string());
                            }
                        });
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    view.terrain.terrain = index;
                }
            }
            let mixes = MixCompleteness::of(def, &set);
            ui.add_space(4.0);
            ui.label(
                theme::muted(format!(
                    "{} of {} mixes beyond the base have a tile",
                    mixes.have, mixes.total
                ))
                .size(12.0),
            );
        });
    });
}

pub fn terrain_color(index: usize) -> Color32 {
    theme::TERRAIN[index % theme::TERRAIN.len()]
}
