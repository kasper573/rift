use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, Pos2, Rect as ERect, RichText, Sense, Stroke, Ui, Vec2};
use engine::systems::area::tileset::TilesetDef;
use strum::VariantArray;

use super::{NewTerrainSetDialog, OpenTileset, TERRAINS_PART, sheet, tile_mut, try_change_tileset};
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::ui::dialogs::{self, DialogAnswer};
use crate::ui::{theme, widgets};
use crate::views::area::palette::terrain_color;
use crate::{forms, rows};

const CLOSEUP: f32 = 192.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, VariantArray)]
pub enum TileCorner {
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

impl TileCorner {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn code(self) -> &'static str {
        match self {
            TileCorner::NorthWest => "nw",
            TileCorner::NorthEast => "ne",
            TileCorner::SouthWest => "sw",
            TileCorner::SouthEast => "se",
        }
    }

    pub fn at(rect: ERect, pos: Pos2) -> TileCorner {
        let east = pos.x >= rect.center().x;
        let south = pos.y >= rect.center().y;
        match (east, south) {
            (false, false) => TileCorner::NorthWest,
            (true, false) => TileCorner::NorthEast,
            (false, true) => TileCorner::SouthWest,
            (true, true) => TileCorner::SouthEast,
        }
    }

    fn name(self) -> &'static str {
        match self {
            TileCorner::NorthWest => "North-west",
            TileCorner::NorthEast => "North-east",
            TileCorner::SouthWest => "South-west",
            TileCorner::SouthEast => "South-east",
        }
    }

    pub fn quarter(self, rect: ERect) -> ERect {
        let min = Pos2::new(
            if self.west() {
                rect.min.x
            } else {
                rect.center().x
            },
            if self.north() {
                rect.min.y
            } else {
                rect.center().y
            },
        );
        ERect::from_min_size(min, rect.size() / 2.0)
    }

    pub fn west(self) -> bool {
        matches!(self, TileCorner::NorthWest | TileCorner::SouthWest)
    }

    fn north(self) -> bool {
        matches!(self, TileCorner::NorthWest | TileCorner::NorthEast)
    }

    fn centre(self, rect: ERect, inset: f32) -> Pos2 {
        Pos2::new(
            if self.west() {
                rect.min.x + inset
            } else {
                rect.max.x - inset
            },
            if self.north() {
                rect.min.y + inset
            } else {
                rect.max.y - inset
            },
        )
    }
}

pub struct MixCompleteness {
    pub have: usize,
    pub total: usize,
    pub missing: Vec<String>,
}

impl MixCompleteness {
    pub fn of(def: &TilesetDef, set: &str) -> MixCompleteness {
        let terrains = def.terrains.get(set).map_or(&[][..], Vec::as_slice);
        let by_mix = tiles_by_mix(def, set);
        let mut have = 0;
        let mut missing = Vec::new();
        for mix in all_mixes(terrains.len()).skip(1) {
            if by_mix.contains_key(&mix_names(mix, terrains)) {
                have += 1;
            } else {
                missing.push(mix_code(mix));
            }
        }
        MixCompleteness {
            have,
            total: have + missing.len(),
            missing,
        }
    }
}

pub struct TerrainSetTiles {
    pub tiles: usize,
    pub twins: usize,
}

impl TerrainSetTiles {
    pub fn of(def: &TilesetDef, set: &str) -> TerrainSetTiles {
        let by_mix = tiles_by_mix(def, set);
        TerrainSetTiles {
            tiles: by_mix.values().map(Vec::len).sum(),
            twins: by_mix
                .values()
                .filter(|tiles| tiles.len() > 1)
                .map(Vec::len)
                .sum(),
        }
    }
}

pub fn current_set(editor: &mut Editor, tileset: &OpenTileset) -> Option<String> {
    let view = editor.views.tileset.view_mut(&tileset.key);
    if view
        .set
        .as_ref()
        .is_none_or(|set| !tileset.def.terrains.contains_key(set))
    {
        match tileset.def.terrains.keys().next() {
            Some(first) => view.pick_set(first.clone()),
            None => view.set = None,
        }
    }
    view.set.clone()
}

pub fn legend(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let Some(set) = current_set(editor, tileset) else {
        widgets::hint(ui, "no terrain sets yet");
        return;
    };
    let view = editor.views.tileset.view_mut(&tileset.key);
    widgets::combo_box(("terrain-set", &tileset.key))
        .selected_text(theme::mono(&set))
        .width(110.0)
        .show_ui(ui, |ui| {
            for option in tileset.def.terrains.keys() {
                if ui
                    .selectable_label(*option == set, theme::mono(option))
                    .clicked()
                {
                    view.pick_set(option.clone());
                }
            }
        });
    ui.add_space(8.0);
    for (index, terrain) in tileset.terrains(&set).iter().enumerate() {
        let on = view.terrain == index;
        let response = ui
            .horizontal(|ui| {
                let (swatch, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                shape(ui.painter(), swatch.center(), 6.0, index, true);
                if on {
                    ui.painter().rect_stroke(
                        swatch,
                        2.0,
                        Stroke::new(1.5, Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                }
                ui.label(if on {
                    theme::strong(terrain)
                } else {
                    theme::muted(terrain)
                });
                if index == 0 {
                    ui.label(theme::faint("base").size(11.0));
                }
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            view.terrain = index;
        }
    }
    ui.horizontal(|ui| {
        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        ui.painter()
            .circle_stroke(swatch.center(), 5.0, Stroke::new(1.0, theme::MUTED));
        ui.label(theme::muted("another set"));
    });
}

pub fn shape(painter: &egui::Painter, center: Pos2, radius: f32, index: usize, filled: bool) {
    let color = terrain_color(index);
    let stroke = Stroke::new(1.5, color);
    let fill = if filled { color } else { Color32::TRANSPARENT };
    match index % 4 {
        0 => {
            let rect = ERect::from_center_size(center, Vec2::splat(radius * 1.6));
            painter.rect(rect, 1.0, fill, stroke, egui::StrokeKind::Inside);
        }
        1 => {
            painter.circle(center, radius * 0.85, fill, stroke);
        }
        2 => {
            let points = vec![
                center + Vec2::new(0.0, -radius),
                center + Vec2::new(radius, radius * 0.8),
                center + Vec2::new(-radius, radius * 0.8),
            ];
            painter.add(egui::Shape::convex_polygon(points, fill, stroke));
        }
        _ => {
            let points = vec![
                center + Vec2::new(0.0, -radius),
                center + Vec2::new(radius, 0.0),
                center + Vec2::new(0.0, radius),
                center + Vec2::new(-radius, 0.0),
            ];
            painter.add(egui::Shape::convex_polygon(points, fill, stroke));
        }
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let Some(set) = current_set(editor, tileset) else {
        widgets::hint(ui, "This tileset declares no terrain set.");
        return;
    };
    let key = tileset.key.as_str();
    let terrains = tileset.terrains(&set);
    let in_other_sets: BTreeSet<u32> = tileset
        .def
        .tiles
        .iter()
        .filter(|(_, data)| data.corners.keys().any(|other| *other != set))
        .map(|(tile, _)| *tile)
        .collect();
    let closeup = editor.views.tileset.view(key).closeup;
    let sheet_h = (ui.available_height() - CLOSEUP - 80.0).max(120.0);
    let mut clicked = None;
    ui.allocate_ui(Vec2::new(ui.available_width(), sheet_h), |ui| {
        let canvas = sheet::canvas(editor, ui, tileset, |painter, tile, cell| {
            let Some(corners) = tileset.corners(tile, &set) else {
                painter.rect_filled(cell, 0.0, Color32::from_black_alpha(120));
                if in_other_sets.contains(&tile) {
                    painter.circle_stroke(
                        cell.center(),
                        (cell.width() * 0.2).max(2.0),
                        Stroke::new(1.0, theme::MUTED),
                    );
                }
                return;
            };
            painter.rect_stroke(
                cell,
                0.0,
                Stroke::new(1.0, Color32::from_white_alpha(80)),
                egui::StrokeKind::Inside,
            );
            let quarter = cell.width() / 4.0;
            for corner in TileCorner::VARIANTS {
                let terrain = &corners[corner.index()];
                if let Some(index) = terrains.iter().position(|known| known == terrain) {
                    let centre = corner.centre(cell, quarter);
                    shape(painter, centre, (quarter * 0.6).max(2.0), index, index != 0);
                }
            }
            if closeup == Some(tile) {
                painter.rect_stroke(
                    cell,
                    0.0,
                    Stroke::new(2.0, theme::SELECTION),
                    egui::StrokeKind::Inside,
                );
            }
        });
        if canvas.response.clicked()
            && let Some(pos) = canvas.response.interact_pointer_pos()
            && let Some(tile) = canvas.layout.tile_at(&tileset.sheet, pos)
        {
            let cell = canvas.layout.tile_rect(&tileset.sheet, tile);
            clicked = Some((tile, TileCorner::at(cell, pos)));
        }
    });
    if let Some((tile, corner)) = clicked {
        let view = editor.views.tileset.view_mut(key);
        view.closeup = Some(tile);
        view.tile = Some(tile);
        let terrain = view.terrain;
        set_corner(editor, tileset, &set, tile, corner, terrain);
        super::select_part(editor, key, TERRAINS_PART);
    }
    if !ui.ctx().egui_wants_keyboard_input()
        && let Some(index) = widgets::digit_pressed(ui)
        && index < terrains.len()
    {
        editor.views.tileset.view_mut(key).terrain = index;
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(theme::muted(
            "Clicking a corner sets the terrain picked above;",
        ));
        for index in 0..terrains.len().min(9) {
            widgets::key_hint(ui, &key_of(index));
        }
        ui.label(theme::muted(
            "pick one. The area brush reads these corners.",
        ));
    });
    ui.add_space(10.0);
    close_up(editor, ui, tileset, &set);
}

pub fn set_corner(
    editor: &mut Editor,
    tileset: &OpenTileset,
    set: &str,
    tile: u32,
    corner: TileCorner,
    terrain: usize,
) {
    let terrains = tileset.terrains(set);
    let Some(name) = terrains.get(terrain) else {
        return;
    };
    let mut corners = tileset
        .corners(tile, set)
        .cloned()
        .unwrap_or_else(|| std::array::from_fn(|_| terrains[0].clone()));
    if corners[corner.index()] == *name {
        return;
    }
    corners[corner.index()] = name.clone();
    let edit = ContentEdit::new(format!(
        "Corner {} of tile {tile} is {name} in {set} of {}",
        corner.code(),
        tileset.key
    ));
    super::change_tileset(editor, &tileset.key, edit, |def| {
        tile_mut(def, tile).corners.insert(set.to_owned(), corners);
    });
}

pub fn remove_set(editor: &mut Editor, tileset: &OpenTileset, set: &str) {
    let edit = ContentEdit::new(format!("Remove terrain set {set} from {}", tileset.key));
    super::change_tileset(editor, &tileset.key, edit, |def| {
        def.terrains.remove(set);
        for data in def.tiles.values_mut() {
            data.corners.remove(set);
        }
    });
}

pub fn mixes(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let Some(set) = current_set(editor, tileset) else {
        widgets::hint(ui, "no terrain set");
        return;
    };
    let terrains = tileset.terrains(&set);
    if terrains.is_empty() {
        return;
    }
    let by_mix = tiles_by_mix(&tileset.def, &set);
    let legend = terrains
        .iter()
        .enumerate()
        .map(|(index, terrain)| format!("{terrain} ({})", key_of(index)))
        .collect::<Vec<_>>()
        .join(" and ");
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::strong(&set));
        ui.label(theme::muted(format!(
            "· every mix of {legend}, corners read nw ne sw se"
        )));
    });
    ui.add_space(4.0);
    let per_row = ((ui.available_width() - 16.0) / 58.0).floor().max(1.0) as usize;
    egui::ScrollArea::vertical()
        .id_salt(("mixes", &tileset.key))
        .show(ui, |ui| {
            egui::Grid::new(("mixes-grid", &tileset.key))
                .spacing(Vec2::new(8.0, 6.0))
                .show(ui, |ui| {
                    for (index, mix) in all_mixes(terrains.len()).enumerate() {
                        let tiles = by_mix.get(&mix_names(mix, terrains));
                        ui.vertical(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                            mix_cell(ui.painter_at(rect), rect, mix, tiles, tileset);
                            ui.label(theme::faint(mix_code(mix)).size(10.5).monospace());
                        });
                        if (index + 1) % per_row == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
}

pub fn new_set_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    tileset: &OpenTileset,
    mut dialog: NewTerrainSetDialog,
) -> Option<NewTerrainSetDialog> {
    let mut answer = None;
    dialogs::card(ctx, "new-terrain-set", 420.0, |ui| {
        ui.label(RichText::new("New terrain set").size(16.0).strong());
        ui.add_space(6.0);
        ui.label(theme::muted(
            "Two to eight terrains; the first is the base an empty cell reads as.",
        ));
        ui.add_space(8.0);
        widgets::kv(ui, "set", |ui| {
            let response = forms::boxed_input(ui, None, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.name)
                        .desired_width(220.0)
                        .font(egui::TextStyle::Monospace),
                )
            });
            widgets::default_focus(ui, &response);
        });
        widgets::kv(ui, "terrains", |ui| {
            forms::boxed_input(ui, None, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.terrains)
                        .desired_width(220.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("floor, rug"),
                )
            });
        });
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(10.0);
        answer = dialogs::answer_row(ui, "Add set");
    });
    match answer {
        None => Some(dialog),
        Some(DialogAnswer::Cancel) => None,
        Some(DialogAnswer::Confirm) => match add_set(editor, tileset, &dialog) {
            Ok(()) => None,
            Err(error) => Some(NewTerrainSetDialog {
                error: Some(error),
                ..dialog
            }),
        },
    }
}

fn add_set(
    editor: &mut Editor,
    tileset: &OpenTileset,
    dialog: &NewTerrainSetDialog,
) -> Result<(), String> {
    let set = rows::key_from_name(&dialog.name);
    let terrains: Vec<String> = dialog
        .terrains
        .split(',')
        .map(rows::key_from_name)
        .filter(|terrain| !terrain.is_empty())
        .collect();
    if set.is_empty() {
        return Err("a set needs a name".to_owned());
    }
    if tileset.def.terrains.contains_key(&set) {
        return Err(format!("{set} is already a set of this tileset"));
    }
    if !(2..=8).contains(&terrains.len()) {
        return Err("two to eight terrains, separated by commas".to_owned());
    }
    let edit = ContentEdit::new(format!("New terrain set {set} on {}", tileset.key));
    try_change_tileset(editor, &tileset.key, edit, |def| {
        def.terrains.insert(set.clone(), terrains);
    })?;
    editor.views.tileset.view_mut(&tileset.key).pick_set(set);
    Ok(())
}

fn close_up(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset, set: &str) {
    let Some(tile) = editor.views.tileset.view(&tileset.key).closeup else {
        widgets::hint(ui, "Click a tile's corner to see it close up here.");
        return;
    };
    let terrains = tileset.terrains(set);
    let corners = tileset.corners(tile, set);
    let mut clicked = None;
    ui.horizontal_top(|ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(CLOSEUP), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::INPUT);
        tileset.sheet.draw(&painter, rect, tile);
        for corner in TileCorner::VARIANTS {
            let centre = corner.centre(rect, CLOSEUP / 8.0);
            let index = corners
                .and_then(|corners| {
                    terrains
                        .iter()
                        .position(|terrain| *terrain == corners[corner.index()])
                })
                .unwrap_or(0);
            painter.circle_filled(centre, 13.0, Color32::from_black_alpha(120));
            shape(&painter, centre, 9.0, index, index != 0);
            let (offset, align) = if corner.west() {
                (18.0, egui::Align2::LEFT_CENTER)
            } else {
                (-18.0, egui::Align2::RIGHT_CENTER)
            };
            painter.text(
                centre + Vec2::new(offset, 0.0),
                align,
                corner.code(),
                egui::FontId::monospace(11.0),
                Color32::from_white_alpha(160),
            );
        }
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            clicked = Some(TileCorner::at(rect, pos));
        }
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::strong(format!("tile {tile}, close up.")));
                ui.label(theme::muted(sentence(corners)));
                ui.label(theme::muted("Click a corner here or above to set it."));
            });
        });
    });
    if let Some(corner) = clicked {
        let terrain = editor.views.tileset.view(&tileset.key).terrain;
        set_corner(editor, tileset, set, tile, corner, terrain);
    }
}

fn sentence(corners: Option<&[String; 4]>) -> String {
    let Some(corners) = corners else {
        return "Not in this set: every corner reads as the base.".to_owned();
    };
    let mut tally: Vec<(&String, Vec<&str>)> = Vec::new();
    for corner in TileCorner::VARIANTS {
        let terrain = &corners[corner.index()];
        match tally.iter_mut().find(|(seen, _)| *seen == terrain) {
            Some((_, list)) => list.push(corner.name()),
            None => tally.push((terrain, vec![corner.name()])),
        }
    }
    tally.sort_by_key(|(_, list)| list.len());
    let Some((rest, _)) = tally.pop() else {
        return String::new();
    };
    if tally.is_empty() {
        return format!("Every corner is {rest}.");
    }
    let parts: Vec<String> = tally
        .iter()
        .map(|(terrain, list)| {
            let verb = if list.len() == 1 { "is" } else { "are" };
            format!("{} {verb} {terrain}", list.join(" and "))
        })
        .collect();
    format!("{}, the rest {rest}.", parts.join(", "))
}

fn mix_cell(
    painter: egui::Painter,
    rect: ERect,
    mix: [usize; 4],
    tiles: Option<&Vec<u32>>,
    tileset: &OpenTileset,
) {
    painter.rect_filled(rect, 3.0, theme::INPUT);
    if mix == [0; 4] {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "base",
            egui::FontId::proportional(11.0),
            theme::MUTED,
        );
        return;
    }
    let Some(tiles) = tiles else {
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0, theme::DANGER),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "–",
            egui::FontId::proportional(14.0),
            theme::DANGER,
        );
        return;
    };
    tileset.sheet.draw(&painter, rect.shrink(2.0), tiles[0]);
    if tiles.len() > 1 {
        painter.text(
            rect.right_bottom() - Vec2::new(2.0, 2.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("×{}", tiles.len()),
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
    }
}

fn all_mixes(terrains: usize) -> impl Iterator<Item = [usize; 4]> {
    (0..terrains.pow(4)).map(move |index| {
        let mut rest = index;
        let mut mix = [0; 4];
        for slot in mix.iter_mut().rev() {
            *slot = rest % terrains;
            rest /= terrains;
        }
        mix
    })
}

fn mix_names(mix: [usize; 4], terrains: &[String]) -> [String; 4] {
    std::array::from_fn(|corner| terrains[mix[corner]].clone())
}

fn mix_code(mix: [usize; 4]) -> String {
    mix.iter().map(|index| key_of(*index)).collect()
}

fn key_of(terrain: usize) -> String {
    (terrain + 1).to_string()
}

fn tiles_by_mix<'a>(def: &'a TilesetDef, set: &str) -> BTreeMap<&'a [String; 4], Vec<u32>> {
    let mut by_mix: BTreeMap<&[String; 4], Vec<u32>> = BTreeMap::new();
    for (tile, data) in &def.tiles {
        if let Some(corners) = data.corners.get(set) {
            by_mix.entry(corners).or_default().push(*tile);
        }
    }
    by_mix
}
