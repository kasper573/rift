use engine::core::units::TilePos;
use engine::systems::area::{self, Pictured};
use ui::egui;
use ui::egui::{Rect as ERect, Vec2};
use ui::theme;
use ui::words;

use super::{AreaFrame, placement};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;

pub(super) fn status_bar(
    editor: &mut Editor,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
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
        left.push(format!("{}, {} m", cell.x, cell.y));
        if frame.in_map(tile) {
            let area = frame.area();
            let mut what = if area.grid.walkable(tile) {
                "walkable".to_owned()
            } else {
                "blocked".to_owned()
            };
            let material = area.paint.dominant(cell);
            let ground = area.terrain.materials[material].get(&frame.content);
            what.push_str(&format!(" · {}", ground.name));
            let height = area.heights.height_at(tile).0;
            if height.abs() >= 0.05 {
                what.push_str(&format!(" · {height:+.1} m"));
            }
            if let Some(surface) = area.footstep_at(cell) {
                what.push_str(&format!(" · {}", file_title(surface.path(&frame.content))));
            }
            left.push(what);
        }
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
    let quiet = view.in_place.is_empty() && view.show_only.is_none();
    if whole_map_visible && quiet && matches!(frame.pictured, Pictured::Area(_)) {
        let size = frame.size();
        let text = format!(
            "{} · {} × {} m · zoom {}%",
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
            super::nodes::modules(&id.get(&frame.content).scene).len()
        })
}
