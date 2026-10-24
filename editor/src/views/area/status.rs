use engine::core::units::{Cell, TilePos};
use engine::systems::area::{self, Pictured};
use ui::egui;
use ui::egui::{Rect as ERect, Vec2};
use ui::theme;
use ui::words;

use super::{AreaFrame, AreaTool, Armed, placement};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;

pub(super) fn status_bar(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let armed = super::armed(editor, frame);
    let view = editor.views.area.view(view_key);
    let mut left: Vec<String> = Vec::new();
    if let Some(place) = frame.in_place.last() {
        let parts = part_count(frame, &place.blueprint);
        let picked = placement::selected_in_place(editor, &place.blueprint).len();
        let title = file_title(&place.blueprint);
        match picked {
            0 => left.push(format!("{title} · {parts} parts")),
            picked => left.push(format!("{title} · {picked} of {parts} parts selected")),
        }
        left.push(format!(
            "double-click a part to open it · {} goes up one level",
            EditorKey::Cancel.name()
        ));
    } else if let Pictured::Blueprint(_) = frame.pictured {
        let blueprint = &frame.bundle;
        let parts = part_count(frame, blueprint);
        left.push(format!(
            "{} · {parts} part{}",
            file_title(blueprint),
            words::plural(parts)
        ));
        left.push(
            placement::placement_count(&frame.content, blueprint)
                .trim_end_matches('.')
                .to_lowercase(),
        );
    }
    if let Some(said) = frame.flags.said() {
        let name = match &frame.def {
            super::nodes::SceneBundle::Area(area) => area.name.clone(),
            super::nodes::SceneBundle::Blueprint(_) => file_title(&frame.bundle).to_owned(),
        };
        left.push(format!("{said} in {name}"));
    }
    if let Some(tile) = frame.pointer_tile {
        let cell = tile.cell();
        if armed == Armed::Walk && view.tool.paints() {
            left.push(format!("cell {}, {}", cell.x, cell.y));
            if frame.in_map(tile) {
                let area = frame.area();
                let grid = cell
                    .index(area.grids.size)
                    .and_then(|index| area.grids.walk.get(index))
                    .copied()
                    .flatten();
                left.push(format!(
                    "walk grid · {} · {}",
                    grid.map_or("–".to_owned(), |v| if v {
                        "1".to_owned()
                    } else {
                        "0".to_owned()
                    }),
                    if area.grid.walkable(tile) {
                        "walkable"
                    } else {
                        "blocked"
                    }
                ));
            }
        } else if view.tool == AreaTool::Terrain {
            left.push(format!("corner {}, {}", cell.x, cell.y));
            if let (Some(tileset), Some(set)) = (&view.terrain.tileset, &view.terrain.set) {
                left.push(format!("{set} · {tileset}"));
            }
        } else {
            left.push(format!("tile {}, {}", cell.x, cell.y));
            if matches!(
                view.tool,
                AreaTool::Paint | AreaTool::Fill | AreaTool::Erase | AreaTool::Eyedropper
            ) && let Some(tileset) = &view.pick.tileset
            {
                let layer = match &armed {
                    Armed::Layer { name, .. } => name.clone(),
                    _ => String::new(),
                };
                let picked = view
                    .pick
                    .block
                    .first()
                    .and_then(|row| row.first())
                    .map(|t| format!(" #{t}"))
                    .unwrap_or_default();
                left.push(format!("{layer} · {tileset}{picked}"));
            } else if frame.in_map(tile) {
                let area = frame.area();
                let mut what = if area.grid.walkable(tile) {
                    "walkable".to_owned()
                } else {
                    "blocked".to_owned()
                };
                if let Some(surface) = area.footstep_at(cell) {
                    what.push_str(&format!(" · {}", file_title(surface.path(&frame.content))));
                }
                left.push(what);
            }
        }
    }
    if let Some(stroke) = view.stroke() {
        left.push(format!(
            "stroke: {} {}",
            stroke.cells.len(),
            if view.tool == AreaTool::Terrain {
                "corners"
            } else {
                "cells"
            }
        ));
    }
    if let Some(skipped) = &view.skipped {
        left.push(skipped.clone());
    }
    let zoom = view.zoom;
    editor
        .status
        .right
        .push(format!("zoom {}%", (zoom * 100.0).round()));
    if !left.is_empty() {
        editor.status.left.append(&mut left);
    }
    let whole_map_visible = {
        let visible = frame.canvas.visible();
        let size = frame.size();
        visible.min().x <= 0.0
            && visible.min().y <= 0.0
            && visible.max().x >= size.width
            && visible.max().y >= size.height
    };
    let quiet = view.in_place.is_empty() && view.show_only.is_none() && !view.tool.paints();
    if whole_map_visible && quiet && matches!(frame.pictured, Pictured::Area(_)) {
        let size = frame.size();
        let text = format!(
            "{} · {} × {} · zoom {}%",
            frame.bundle,
            size.width,
            size.height,
            (zoom * 100.0).round()
        );
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(12.0), theme::TEXT);
        let at = frame.canvas.screen.min + Vec2::new(16.0, 12.0);
        let back = ERect::from_min_size(at, galley.size() + Vec2::new(12.0, 6.0));
        ui::slip::paint(painter, back);
        painter.galley(at + Vec2::new(6.0, 3.0), galley, theme::TEXT);
    }
}

fn part_count(frame: &AreaFrame, blueprint: &str) -> usize {
    frame
        .content
        .by_path::<area::blueprint::BlueprintDef>(blueprint)
        .map_or(0, |id| {
            super::nodes::modules(&id.get(&frame.content).scene.nodes).len()
        })
}
