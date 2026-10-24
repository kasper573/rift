use std::collections::HashSet;
use ui::egui;
use ui::ink;
use ui::paint;
use ui::theme;

use engine::core::math::{Pos, Rect, Rng, Size};
use engine::core::units::{Cell, CellPos, TilePos, TileSize};
use engine::systems::area::tileset::TilesetDef;
use engine::systems::area::{self, Flip};
use engine::systems::script::Place;
use ui::egui::{Color32, Stroke as EStroke, Vec2};

use super::{
    AreaFrame, AreaSnapshot, AreaTool, Armed, grid, nodes, overlays, pins, select, terrain,
};
use crate::app::{Editor, PlacePick};
use crate::keymap::EditorKey;
use crate::thumbs;
use crate::user::BrushSettings;
use crate::views::tileset::SheetCell;

#[derive(Clone, Default, Debug)]
pub struct TilePick {
    pub tileset: Option<String>,
    pub block: Vec<Vec<u32>>,
    pub random: Vec<u32>,
    pub anchor: Option<SheetCell>,
}

impl TilePick {
    pub fn is_random(&self, brush: &BrushSettings) -> bool {
        !self.random.is_empty() || brush.random
    }

    pub fn is_empty(&self) -> bool {
        self.block.is_empty() && self.random.is_empty()
    }

    pub fn block_size(&self) -> (usize, usize) {
        (self.block.first().map_or(0, Vec::len), self.block.len())
    }

    pub fn choices(&self) -> Vec<u32> {
        if self.random.is_empty() {
            self.block.iter().flatten().copied().collect()
        } else {
            self.random.clone()
        }
    }

    pub fn tile_at(&self, anchor: CellPos, cell: CellPos, flip_x: bool, flip_y: bool) -> u32 {
        let (w, h) = self.block_size();
        let (w, h) = (w as i32, h as i32);
        let bx = (cell.x - anchor.x).rem_euclid(w);
        let by = (cell.y - anchor.y).rem_euclid(h);
        let bx = if flip_x { w - 1 - bx } else { bx };
        let by = if flip_y { h - 1 - by } else { by };
        self.block[by as usize][bx as usize]
    }
}

#[derive(Clone, Default, Debug)]
pub struct TerrainPick {
    pub tileset: Option<String>,
    pub set: Option<String>,
    pub terrain: usize,
}

pub struct BrushStroke {
    pub draft: grid::GridDraft,
    pub kind: BrushStrokeKind,
    /// The layer of the draft the stroke paints.
    pub layer: usize,
    pub cells: HashSet<CellPos>,
    pub anchor: CellPos,
    /// The cell the brush was last applied at, from which the next one draws a line, so a drag
    /// faster than a cell a frame leaves no gaps.
    pub last: CellPos,
    pub rng: Rng,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushStrokeKind {
    Paint,
    Erase,
    /// Forces walk cells to a value, or hands them back to their tiles.
    Walk(Option<bool>),
    Terrain,
}

/// The layer of `draft` the brushes paint: the one the selection arms, unless it is hidden or
/// locked.
pub fn target_layer(
    editor: &Editor,
    snapshot: &AreaSnapshot,
    draft: &grid::GridDraft,
) -> Option<usize> {
    match super::armed(editor, snapshot) {
        Armed::Layer { pointer, name }
            if !snapshot.flags.hidden(&pointer) && !snapshot.flags.locked(&pointer) =>
        {
            draft.layer_index(&name)
        }
        _ => None,
    }
}

pub(super) fn grid_stroke(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    kind: BrushStrokeKind,
) {
    let response = frame.response;
    let brush = editor.settings.brush.clone();
    // A press that moves too little to drag is only a click once let go, when the pointer no
    // longer says where it was pressed, so the stroke starts on the press itself.
    let pressed = response.drag_started_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary)
        || (response.is_pointer_button_down_on()
            && response.ctx.input(|i| i.pointer.primary_pressed()));
    let down = response.dragged_by(egui::PointerButton::Primary);
    let released = response.drag_stopped_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary);
    let Some(cell) = frame.drag_tile().map(|tile| tile.cell()) else {
        if released {
            finish_stroke(editor, view_key, frame);
        }
        return;
    };
    if pressed && editor.views.area.view(view_key).stroke().is_none() {
        let Some(draft) = grid::GridDraft::own(editor, &frame.bundle) else {
            return;
        };
        let start = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .map_or(cell, |origin| frame.canvas.to_tile(origin).cell());
        let layer = match kind {
            BrushStrokeKind::Walk(_) => 0,
            _ => match target_layer(editor, frame, &draft) {
                Some(layer) => layer,
                None => return,
            },
        };
        let seed = stroke_seed(editor.frame, cell);
        let view = editor.views.area.view_mut(view_key);
        view.skipped = None;
        view.set_stroke(BrushStroke {
            draft,
            kind,
            layer,
            cells: HashSet::new(),
            anchor: start,
            last: start,
            rng: Rng::new(seed),
        });
        apply_brush(editor, view_key, frame, start, &brush);
    }
    if pressed || down {
        let last = editor
            .views
            .area
            .view(view_key)
            .stroke()
            .map(|stroke| stroke.last);
        for step in last.map(|last| cells_after(last, cell)).unwrap_or_default() {
            apply_brush(editor, view_key, frame, step, &brush);
        }
        if let Some(stroke) = editor.views.area.view_mut(view_key).stroke_mut() {
            stroke.last = cell;
        }
    }
    if released {
        finish_stroke(editor, view_key, frame);
    }
}

pub(super) fn apply_brush(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    center: CellPos,
    brush: &BrushSettings,
) {
    let content = frame.content.clone();
    let view = editor.views.area.view_mut(view_key);
    let pick = view.pick.clone();
    let terrain = view.terrain.clone();
    let Some(stroke) = view.stroke_mut() else {
        return;
    };
    let layer = stroke.layer;
    match stroke.kind {
        BrushStrokeKind::Walk(value) => {
            for cell in grid::brush_cells(center, brush.size) {
                if stroke.draft.set_walk(cell, value) {
                    stroke.cells.insert(cell);
                }
            }
        }
        BrushStrokeKind::Erase => {
            for cell in grid::brush_cells(center, brush.size) {
                if stroke.draft.set(layer, cell, None) {
                    stroke.cells.insert(cell);
                }
            }
        }
        BrushStrokeKind::Paint => {
            let Some(tileset) = pick.tileset.clone() else {
                return;
            };
            if pick.is_empty() {
                return;
            }
            let flip = Flip::of(brush.flip_x, brush.flip_y);
            if pick.is_random(brush) {
                let choices = pick.choices();
                for cell in grid::brush_cells(center, brush.size) {
                    if stroke.cells.contains(&cell) {
                        continue;
                    }
                    let tile = stroke.rng.choose(&choices).copied().unwrap_or(0);
                    let entry = stroke.draft.intern(&tileset, tile, flip);
                    if stroke.draft.set(layer, cell, Some(entry)) {
                        stroke.cells.insert(cell);
                    }
                }
            } else {
                for cell in grid::brush_cells(center, brush.size) {
                    let tile = pick.tile_at(stroke.anchor, cell, brush.flip_x, brush.flip_y);
                    let entry = stroke.draft.intern(&tileset, tile, flip);
                    if stroke.draft.set(layer, cell, Some(entry)) {
                        stroke.cells.insert(cell);
                    }
                }
            }
        }
        BrushStrokeKind::Terrain => {
            let (Some(tileset), Some(set)) = (terrain.tileset.clone(), terrain.set.clone()) else {
                return;
            };
            let Some(def) = content
                .by_path::<TilesetDef>(&tileset)
                .map(|id| id.get(&content))
            else {
                return;
            };
            let terrain_brush = terrain::TerrainBrush {
                def,
                tileset: &tileset,
                set: &set,
                terrain: terrain.terrain,
                size: brush.size,
            };
            let kept = terrain::paint(
                &mut stroke.draft,
                layer,
                &terrain_brush,
                center,
                &mut stroke.rng,
                &mut stroke.cells,
            );
            if kept > 0 {
                view.skipped = Some(format!("{kept} cells kept: no tile for that mix"));
            }
        }
    }
}

pub(super) fn finish_stroke(editor: &mut Editor, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view_mut(view_key);
    let Some(stroke) = view.take_stroke() else {
        return;
    };
    if stroke.cells.is_empty() || !stroke.draft.is_touched() {
        return;
    }
    let layer_name = stroke
        .draft
        .layer_names()
        .get(stroke.layer)
        .cloned()
        .unwrap_or_default();
    let label = match stroke.kind {
        BrushStrokeKind::Paint => format!(
            "Paint {} cells of {layer_name} in {}",
            stroke.cells.len(),
            frame.bundle
        ),
        BrushStrokeKind::Erase => format!(
            "Erase {} cells of {layer_name} in {}",
            stroke.cells.len(),
            frame.bundle
        ),
        BrushStrokeKind::Walk(_) => format!(
            "Walk grid: {} cells in {}",
            stroke.cells.len(),
            frame.bundle
        ),
        BrushStrokeKind::Terrain => format!(
            "Terrain: {} cells of {layer_name} in {}",
            stroke.cells.len(),
            frame.bundle
        ),
    };
    let mut draft = stroke.draft;
    if stroke.kind == BrushStrokeKind::Erase {
        draft.compact_palette();
    }
    draft.commit(editor, label);
}

pub(super) fn fill(editor: &mut Editor, view_key: &str, frame: &AreaFrame, cell: CellPos) {
    let view = editor.views.area.view(view_key);
    let pick = view.pick.clone();
    let walking = view.armed == Armed::Walk;
    let walk_value = view.walk_value;
    let brush = editor.settings.brush.clone();
    let Some(mut draft) = grid::GridDraft::own(editor, &frame.bundle) else {
        return;
    };
    let layer = target_layer(editor, frame, &draft);
    let cells = match layer {
        Some(layer) => draft.flood(layer, cell),
        None if walking => draft.flood_walk(cell),
        None => return,
    };
    if cells.is_empty() {
        return;
    }
    let count = cells.len();
    let Some(layer) = layer else {
        for cell in cells {
            draft.set_walk(cell, walk_value);
        }
        draft.commit(
            editor,
            format!("Walk grid: fill {count} cells in {}", frame.bundle),
        );
        return;
    };
    let Some(tileset) = pick.tileset.clone() else {
        editor.toast("Pick a tile first");
        return;
    };
    if pick.is_empty() {
        editor.toast("Pick a tile first");
        return;
    }
    let flip = Flip::of(brush.flip_x, brush.flip_y);
    let mut rng = Rng::new(editor.frame);
    let choices = pick.choices();
    for c in cells {
        let tile = if pick.is_random(&brush) {
            rng.choose(&choices).copied().unwrap_or(0)
        } else {
            pick.tile_at(cell, c, brush.flip_x, brush.flip_y)
        };
        let entry = draft.intern(&tileset, tile, flip);
        draft.set(layer, c, Some(entry));
    }
    let layer_name = draft.layer_names().get(layer).cloned().unwrap_or_default();
    draft.commit(
        editor,
        format!("Fill {count} cells of {layer_name} in {}", frame.bundle),
    );
}

pub(super) fn eyedrop(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    cell: CellPos,
    topmost: bool,
) {
    let grids = frame.picture.ground.clone();
    let armed = match super::armed(editor, frame) {
        Armed::Layer { name, .. } => grids.ground.iter().position(|layer| layer.name == name),
        _ => None,
    };
    let layers: Vec<usize> = match armed {
        Some(layer) if !topmost => vec![layer],
        _ => (0..grids.ground.len()).rev().collect(),
    };
    for layer in layers {
        let Some(index) = cell.index(grids.size) else {
            continue;
        };
        let Some(entry) = grids
            .ground
            .get(layer)
            .and_then(|l| l.cells[index])
            .and_then(|i| grids.palette_keys.get(i as usize))
        else {
            continue;
        };
        let name = grids.ground[layer].name.clone();
        let mut pointer = None;
        nodes::walk(frame.nodes(), &mut |at, node| {
            if nodes::layer_name(node) == Some(name.as_str()) {
                pointer = Some(at.to_owned());
            }
        });
        let view = editor.views.area.view_mut(view_key);
        view.pick = TilePick {
            tileset: Some(entry.tileset.clone()),
            block: vec![vec![entry.tile]],
            random: Vec::new(),
            anchor: None,
        };
        view.set_tool(AreaTool::Paint);
        if let Some(pointer) = pointer {
            view.armed = Armed::Layer {
                pointer: pointer.clone(),
                name,
            };
            nodes::select(editor, &frame.bundle, vec![pointer]);
        }
        editor.settings.brush.flip_x = entry.flip.horizontal();
        editor.settings.brush.flip_y = entry.flip.vertical();
        return;
    }
    editor.toast("Nothing on that layer here");
}

pub(super) fn pick_place(
    editor: &mut Editor,
    painter: &egui::Painter,
    frame: &AreaFrame,
    pick: &PlacePick,
    fine: bool,
) {
    let canvas = frame.canvas;
    let area = frame.area();
    let Some(area_id) = frame.area_id() else {
        return;
    };
    let warps = pins::warp_objects(&frame.content, area_id);
    overlays::cell_runs(
        canvas,
        area.size.grid(),
        |cell| {
            !area.grid.walkable(cell.center()) || warps.iter().any(|shape| shape.covers_cell(cell))
        },
        |run| {
            paint::hatched(
                painter,
                canvas.rect(run),
                ink::wash(theme::BLOCKED, 0.35),
                canvas.scale(),
            )
        },
    );
    pins::draw_arrivals(editor, painter, frame);
    let text = format!(
        "Pick a place for {} {} · click a cell, {} click any point · {} cancels",
        pick.file,
        pick.pointer,
        EditorKey::Unsnapped.name(),
        EditorKey::Cancel.name()
    );
    painter.text(
        canvas.screen.min + Vec2::new(8.0, 8.0),
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(13.0),
        Color32::from_rgb(255, 230, 120),
    );
    let Some(tile) = frame.pointer_tile else {
        return;
    };
    let at = select::snap_to(tile, fine);
    painter.circle_stroke(
        canvas.to_screen(at),
        6.0,
        EStroke::new(2.0, Color32::from_rgb(255, 230, 120)),
    );
    if !frame.response.clicked() {
        return;
    }
    let place = Place { area: area_id, at };
    if let Err(problem) = area::check_place(&frame.content, place) {
        editor.toast(format!("{}: {}", problem.rule, problem.expected));
        return;
    }
    pins::commit_pick(editor, &frame.bundle, at);
}

pub(super) fn paint_preview(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
    cells: &[CellPos],
) {
    let view = editor.views.area.view(view_key);
    let pick = view.pick.clone();
    let (Some(tileset), false) = (pick.tileset.clone(), pick.is_empty()) else {
        return;
    };
    let Some(def) = frame
        .content
        .by_path::<TilesetDef>(&tileset)
        .map(|id| id.get(&frame.content))
    else {
        return;
    };
    let image = def.image.as_str().to_owned();
    let ctx = painter.ctx().clone();
    let Some(project) = &editor.project else {
        return;
    };
    let Some(thumb) = editor.form.thumbnails.image(&ctx, &project.files, &image) else {
        return;
    };
    let columns = (thumb.size[0] as u32 / def.tile.max(1)).max(1);
    let Some(&first) = cells.first() else {
        return;
    };
    let anchor = view.stroke().map_or(first, |s| s.anchor);
    let brush = &editor.settings.brush;
    let choices = pick.choices();
    for cell in cells {
        let tile = if pick.is_random(brush) {
            choices[((cell.x * 7 + cell.y * 13).rem_euclid(choices.len() as i32)) as usize]
        } else {
            pick.tile_at(anchor, *cell, brush.flip_x, brush.flip_y)
        };
        let (x, y) = ((tile % columns) * def.tile, (tile / columns) * def.tile);
        let uv = thumbs::uv(
            thumb.size,
            x as f32,
            y as f32,
            def.tile as f32,
            def.tile as f32,
        );
        let uv = thumbs::flip_uv(uv, brush.flip_x, brush.flip_y);
        let rect = frame.canvas.rect(Rect::new(
            Pos::new(cell.x as f32, cell.y as f32),
            Size::splat(1.0),
        ));
        painter.image(
            thumb.texture.id(),
            rect,
            uv,
            Color32::from_rgba_unmultiplied(255, 255, 255, 180),
        );
    }
}

fn stroke_seed(frame: u64, cell: CellPos) -> u64 {
    frame
        .wrapping_mul(6364136223846793005)
        .wrapping_add((cell.x as i64).wrapping_mul(31).wrapping_add(cell.y as i64) as u64)
}

/// The cells of the line from `from` to `to`, `to` last and `from` left out: what a brush dragged
/// between them crosses.
fn cells_after(from: CellPos, to: CellPos) -> Vec<CellPos> {
    let steps = (to.x - from.x).abs().max((to.y - from.y).abs());
    (1..=steps)
        .map(|step| {
            let t = step as f32 / steps as f32;
            CellPos::new(
                from.x + ((to.x - from.x) as f32 * t).round() as i32,
                from.y + ((to.y - from.y) as f32 * t).round() as i32,
            )
        })
        .collect()
}
