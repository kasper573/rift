use engine::core::math::{Offset, Rect};
use engine::core::units::{Cell, Meters, TilePos};
use engine::systems::area::{AreaModule, AreaModuleKind};
use strum::VariantArray;
use ui::egui;
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Ui, Vec2};
use ui::paint;
use ui::theme;
use ui::words;

use super::brush::{BrushStrokeKind, eyedrop, fill, grid_stroke, paint_preview, pick_place};
use super::modules::{context_menu_at, open_add_menu, show_add_menu};
use super::select::{
    copy_selection, delete_selection, duplicate_selection, move_selection, paste_anchor,
    paste_in_place, select_input, selected_nodes, snapped_delta, start_paste,
};
use super::{
    AreaDrag, AreaFrame, AreaTool, Armed, MAX_ZOOM, MIN_ZOOM, blueprint, grid, nodes, placement,
    shapes, stamp,
};
use crate::app::{Clipboard, Editor};
use crate::keymap::EditorKey;
use crate::tabs::file_title;
use crate::views::navigation::{self, Viewport};

pub fn rail(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let view = editor.views.area.view_mut(view_key);
    let mut switched = None;
    ui.add_space(6.0);
    for &tool in AreaTool::VARIANTS {
        let selected = view.tool == tool;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 34.0), egui::Sense::click());
        let painter = ui.painter();
        if selected {
            painter.rect(
                rect,
                4.0,
                theme::SELECTION,
                egui::Stroke::new(1.0, theme::TEXT),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            paint::row_wash(painter, rect);
        }
        tool_icon(
            painter,
            tool,
            rect.center() - Vec2::new(0.0, 3.0),
            theme::TEXT,
        );
        painter.text(
            rect.right_bottom() - Vec2::new(4.0, 2.0),
            egui::Align2::RIGHT_BOTTOM,
            tool.key().name(),
            egui::FontId::monospace(9.0),
            theme::MUTED,
        );
        if response
            .on_hover_text(format!("{} ({})", tool.as_ref(), tool.key().name()))
            .clicked()
        {
            view.set_tool(tool);
            switched = Some(tool);
        }
        ui.add_space(2.0);
    }
    if let Some(tool) = switched {
        took(editor, view_key, tool);
    }
}

/// What taking a tool brings along: its bottom tab, and for a brush, a layer to paint into.
fn took(editor: &mut Editor, view_key: &str, tool: AreaTool) {
    if let Some(bottom) = tool.bottom_tab() {
        editor.panels.bottom = bottom;
    }
    let walking = editor.views.area.view(view_key).armed == Armed::Walk;
    let walk_brush = matches!(tool, AreaTool::Paint | AreaTool::Fill | AreaTool::Erase);
    if tool.paints() && !(walking && walk_brush) {
        super::arm_a_layer(editor, view_key);
    }
}

fn tool_icon(painter: &egui::Painter, tool: AreaTool, c: Pos2, ink: Color32) {
    let stroke = EStroke::new(1.6, ink);
    let p = |x: f32, y: f32| Pos2::new(c.x + x, c.y + y);
    match tool {
        AreaTool::Select => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p(-5.0, -7.0),
                    p(5.0, 2.0),
                    p(1.0, 2.5),
                    p(3.5, 7.0),
                    p(1.5, 8.0),
                    p(-1.0, 3.5),
                    p(-5.0, 6.0),
                ],
                ink,
                EStroke::NONE,
            ));
        }
        AreaTool::Paint => {
            painter.line_segment([p(-6.0, 6.0), p(3.0, -3.0)], EStroke::new(2.2, ink));
            painter.line_segment([p(3.0, -3.0), p(6.0, -6.0)], EStroke::new(3.5, ink));
            painter.circle_filled(p(-6.0, 6.0), 2.0, ink);
        }
        AreaTool::Terrain => {
            painter.rect_stroke(
                ERect::from_min_max(p(-6.0, -6.0), p(3.0, 3.0)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.rect_filled(ERect::from_min_max(p(-2.0, -2.0), p(7.0, 7.0)), 1.0, ink);
        }
        AreaTool::Fill => {
            painter.add(egui::Shape::closed_line(
                vec![p(-6.0, -1.0), p(0.0, -7.0), p(6.0, -1.0), p(0.0, 5.0)],
                stroke,
            ));
            painter.circle_filled(p(5.5, 5.0), 2.2, ink);
        }
        AreaTool::Erase => {
            painter.line_segment([p(-5.0, -5.0), p(5.0, 5.0)], EStroke::new(2.0, ink));
            painter.line_segment([p(-5.0, 5.0), p(5.0, -5.0)], EStroke::new(2.0, ink));
        }
        AreaTool::Eyedropper => {
            painter.line_segment([p(-6.0, 6.0), p(2.0, -2.0)], EStroke::new(2.0, ink));
            painter.circle_filled(p(4.0, -4.0), 3.0, ink);
        }
        AreaTool::Stamp => {
            painter.rect_filled(ERect::from_min_max(p(-7.0, 3.0), p(7.0, 7.0)), 1.0, ink);
            painter.rect_filled(ERect::from_min_max(p(-2.0, -3.0), p(2.0, 3.0)), 0.0, ink);
            painter.circle_filled(p(0.0, -5.0), 3.0, ink);
        }
        AreaTool::Module => {
            painter.line_segment([p(-6.0, 0.0), p(6.0, 0.0)], EStroke::new(2.2, ink));
            painter.line_segment([p(0.0, -6.0), p(0.0, 6.0)], EStroke::new(2.2, ink));
        }
    }
}

pub fn input(
    editor: &mut Editor,
    ui: &mut Ui,
    painter: &egui::Painter,
    view_key: &str,
    frame: &AreaFrame,
) {
    let response = frame.response;
    let modifiers = ui.input(|i| i.modifiers);
    let view = editor.views.area.view_mut(view_key);
    let mut viewport = Viewport {
        screen: frame.canvas.screen,
        center: view.center,
        zoom: view.zoom,
    };
    let panning = navigation::zoom_and_pan(ui, response, &mut viewport, (MIN_ZOOM, MAX_ZOOM));
    view.center = viewport.center;
    view.zoom = viewport.zoom;
    if panning {
        return;
    }
    if let Some(pick) = editor.pick.clone() {
        pick_place(
            editor,
            painter,
            frame,
            &pick,
            EditorKey::Unsnapped.held_among(modifiers),
        );
        return;
    }
    let view = editor.views.area.view(view_key);
    let tool = view.tool;
    let walking = view.armed == Armed::Walk;
    let walk_locked = walking && view.walk_locked;
    let walk_value = view.walk_value;
    if response.secondary_clicked() {
        context_menu_at(editor, view_key, frame);
    }
    match tool {
        AreaTool::Select => select_input(editor, ui, view_key, frame, modifiers),
        _ if walk_locked && tool.paints() => {}
        AreaTool::Paint if walking => {
            grid_stroke(editor, view_key, frame, BrushStrokeKind::Walk(walk_value))
        }
        AreaTool::Erase if walking => {
            grid_stroke(editor, view_key, frame, BrushStrokeKind::Walk(None))
        }
        AreaTool::Paint => grid_stroke(editor, view_key, frame, BrushStrokeKind::Paint),
        AreaTool::Erase => grid_stroke(editor, view_key, frame, BrushStrokeKind::Erase),
        AreaTool::Terrain => grid_stroke(editor, view_key, frame, BrushStrokeKind::Terrain),
        AreaTool::Fill => {
            if response.clicked()
                && let Some(cell) = frame.pointer_cell()
            {
                fill(editor, view_key, frame, cell);
            }
        }
        AreaTool::Eyedropper => {
            if response.clicked()
                && let Some(cell) = frame.pointer_cell()
            {
                eyedrop(
                    editor,
                    view_key,
                    frame,
                    cell,
                    EditorKey::Topmost.held_among(modifiers),
                );
            }
        }
        AreaTool::Stamp => stamp::stamp_input(editor, ui, view_key, frame),
        AreaTool::Module => {
            if response.clicked()
                && let Some(tile) = frame.pointer_tile
            {
                let view = editor.views.area.view_mut(view_key);
                view.cursor_cell = Some(tile.cell());
                match view.placing.take() {
                    Some(kind) => {
                        view.set_tool(AreaTool::Select);
                        super::modules::add_module(editor, view_key, frame, kind, tile);
                    }
                    None => open_add_menu(ui, editor, view_key, frame, tile),
                }
            }
        }
    }
    show_add_menu(ui, editor, view_key, frame);
}

pub fn overlay(editor: &mut Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let canvas = frame.canvas;
    let brush = editor.settings.brush.clone();
    let view = editor.views.area.view(view_key);
    let accent = Color32::from_rgb(255, 230, 120);
    match view.drag() {
        Some(AreaDrag::Box { from }) => {
            if let Some(tile) = frame.pointer_tile {
                painter.rect_stroke(
                    canvas.rect(Rect::from_points([*from, tile])),
                    0.0,
                    EStroke::new(1.0, accent),
                    egui::StrokeKind::Inside,
                );
            }
        }
        Some(AreaDrag::Move { from, moved: true }) => {
            if let Some(tile) = frame.pointer_tile {
                let alt = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
                let (dx, dy) = snapped_delta(*from, tile, alt);
                for node in selected_nodes(editor, frame) {
                    if let Some(bounds) = frame.module_bounds(&node) {
                        let moved = bounds.translate(Offset::new(dx, dy));
                        painter.rect_stroke(
                            canvas.rect(moved),
                            0.0,
                            EStroke::new(1.5, accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(listener) = view.listener {
        let at = canvas.to_screen(listener);
        painter.circle_stroke(at, 8.0, EStroke::new(2.0, Color32::from_rgb(170, 110, 255)));
        painter.text(
            at + Vec2::new(10.0, -8.0),
            egui::Align2::LEFT_TOP,
            "listening",
            egui::FontId::proportional(11.0),
            Color32::from_rgb(170, 110, 255),
        );
    }
    if let Some(ghost) = &view.paste
        && let Some(tile) = frame.pointer_tile
    {
        let at = paste_anchor(tile);
        let mut whole: Option<Rect<Meters>> = None;
        let ghosts = nodes::modules(&ghost.nodes);
        for module in &ghosts {
            let mut placed = module.module().clone();
            shapes::shift(&mut placed, at.x, at.y);
            if let Some(bounds) = shapes::bounds(&placed, None) {
                let kind = placed.kind();
                let color = super::overlays::module_color(Some(kind));
                let rect = canvas.rect(bounds);
                paint::dashed_rect(painter, rect, EStroke::new(1.5, color), 6.0, 4.0);
                if matches!(
                    kind,
                    AreaModuleKind::Population | AreaModuleKind::Resident | AreaModuleKind::Object
                ) {
                    frame.labels.chip(
                        rect.min + Vec2::new(0.0, -4.0),
                        shapes::label(&placed, frame.reader.as_ref()),
                        color,
                    );
                }
                whole = Some(whole.map_or(bounds, |w| w.union(&bounds)));
            }
        }
        if let Some(whole) = whole {
            let rect = canvas.rect(whole);
            paint::dashed_rect(
                painter,
                rect,
                EStroke::new(1.0, Color32::from_rgba_unmultiplied(255, 120, 160, 160)),
                8.0,
                6.0,
            );
        }
        frame.labels.chip(
            canvas.to_screen(tile) + Vec2::new(16.0, 8.0),
            format!(
                "{} module{} from {}",
                ghosts.len(),
                words::plural(ghosts.len()),
                file_title(&ghost.clip.from)
            ),
            Color32::WHITE,
        );
        editor
            .status
            .left
            .push(format!("pasting into area/{}", frame.bundle));
        editor.status.left.push(format!(
            "click or {} places · {} cancels",
            EditorKey::Confirm.name(),
            EditorKey::Cancel.name()
        ));
        editor.status.left.push(format!(
            "{} pastes at the same tiles",
            EditorKey::PasteInPlace.name()
        ));
    }
    let view = editor.views.area.view(view_key);
    let Some(cell) = frame.pointer_cell() else {
        return;
    };
    let walking = view.armed == Armed::Walk;
    let covered = match &view.armed {
        Armed::Layer { name, .. } if view.tool.paints() => frame.covering(cell, name),
        _ => None,
    };
    if let Some(placement) = &covered {
        under_placement_tip(frame, placement, &view.armed);
    }
    match view.tool {
        AreaTool::Paint | AreaTool::Erase => {
            let cells = grid::brush_cells(cell, brush.size);
            if let Some(rect) = grid::rect_of_cells(&cells) {
                let color = if view.tool == AreaTool::Erase || walking {
                    Color32::from_rgb(255, 120, 120)
                } else {
                    accent
                };
                if covered.is_some() {
                    paint::dashed_rect(
                        painter,
                        canvas.rect(rect),
                        EStroke::new(1.5, color),
                        5.0,
                        3.0,
                    );
                } else {
                    painter.rect_stroke(
                        canvas.rect(rect),
                        0.0,
                        EStroke::new(1.5, color),
                        egui::StrokeKind::Inside,
                    );
                }
                if view.tool == AreaTool::Paint && !walking {
                    paint_preview(editor, painter, view_key, frame, &cells);
                }
            }
        }
        AreaTool::Terrain => {
            let cells = grid::brush_cells(cell, brush.size);
            if let Some(rect) = grid::rect_of_cells(&cells) {
                let outline = rect.inflate(0.5, 0.5);
                painter.rect_stroke(
                    canvas.rect(outline),
                    0.0,
                    EStroke::new(1.5, Color32::from_rgb(140, 220, 140)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        AreaTool::Module if view.placing.is_some() => {
            let kind = view.placing.expect("a kind being placed");
            let color = super::overlays::module_color(Some(kind));
            let rect = canvas.rect(super::modules::template_bounds(kind, cell));
            paint::dashed_rect(painter, rect, EStroke::new(1.5, color), 6.0, 4.0);
            frame.labels.chip(
                rect.min + Vec2::new(0.0, -4.0),
                format!("new {}", kind.tag().replace('_', " ")),
                color,
            );
        }
        AreaTool::Fill | AreaTool::Eyedropper | AreaTool::Module => {
            painter.rect_stroke(
                canvas.rect(cell.bounds()),
                0.0,
                EStroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        AreaTool::Stamp => stamp::stamp_overlay(editor, painter, view_key, frame),
        AreaTool::Select => {
            if let Some(cursor) = view.cursor_cell
                && !painter.ctx().text_edit_focused()
            {
                painter.rect_stroke(
                    canvas.rect(cursor.bounds()),
                    0.0,
                    EStroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90)),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

pub fn keys(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let input = ui.input(|i| i.clone());
    let pressed = |key: EditorKey| key.pressed(&input);
    if editor.pick.is_some() {
        if pressed(EditorKey::Cancel) {
            editor.pick = None;
            editor.toast("Pick cancelled");
        }
        return;
    }
    if pressed(EditorKey::BrushSmaller) {
        editor.update_brush(|brush| brush.size = brush.size.saturating_sub(1).max(1));
    }
    if pressed(EditorKey::BrushBigger) {
        editor.update_brush(|brush| brush.size = (brush.size + 1).min(9));
    }
    let mut switched = None;
    {
        let view = editor.views.area.view_mut(view_key);
        for &tool in AreaTool::VARIANTS {
            if pressed(tool.key()) {
                view.set_tool(tool);
                switched = Some(tool);
            }
        }
        if pressed(EditorKey::WalkOverlay) {
            view.walk_overlay = !view.walk_overlay;
        }
        if view.tool == AreaTool::Terrain
            && let Some(index) = EditorKey::Pick.digit(&input)
        {
            view.terrain.terrain = index;
        }
    }
    if let Some(tool) = switched {
        took(editor, view_key, tool);
    }
    let view_tool = editor.views.area.view(view_key).tool;
    let flip_across = pressed(EditorKey::FlipHorizontal);
    if flip_across || pressed(EditorKey::FlipVertical) {
        let horizontal = flip_across;
        match view_tool {
            AreaTool::Paint | AreaTool::Fill => {
                editor.update_brush(|brush| {
                    if horizontal {
                        brush.flip_x = !brush.flip_x;
                    } else {
                        brush.flip_y = !brush.flip_y;
                    }
                });
            }
            AreaTool::Stamp => stamp::flip_stamp(editor, view_key, horizontal),
            _ => {
                let flipped = selected_nodes(editor, frame);
                if !flipped.is_empty() {
                    nodes::change_scene(editor, &frame.bundle, |scene| {
                        for node in &flipped {
                            shapes::flip_field(
                                &mut nodes::module_node_mut(scene, node)?.module,
                                horizontal,
                            );
                        }
                        Some(format!("Flip {} modules", flipped.len()))
                    });
                }
            }
        }
    }
    if pressed(EditorKey::Cancel) {
        let view = editor.views.area.view_mut(view_key);
        if !view.cancel_gesture() && view.paste.take().is_none() && view.show_only.take().is_none()
        {
            view.in_place.pop();
            editor.selection.parts.clear();
        }
        return;
    }
    if pressed(EditorKey::Confirm)
        && let [node] = selected_nodes(editor, frame).as_slice()
        && frame
            .module(node)
            .is_some_and(|module| matches!(module, AreaModule::Blueprint(_)))
    {
        placement::edit_in_place(editor, view_key, frame, node);
    }
    if pressed(EditorKey::Delete) {
        delete_selection(editor, frame);
    }
    let (copied, pasted) = ui.input(|i| {
        (
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy)),
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_))),
        )
    });
    if copied || pressed(EditorKey::Copy) {
        copy_selection(editor, frame);
        if let Some(clip) = editor.clipboard.as_ref().and_then(Clipboard::scene) {
            ui.ctx().copy_text(clip.nodes.text());
        }
    }
    if pressed(EditorKey::PasteInPlace) {
        paste_in_place(editor, frame);
    } else if pasted || pressed(EditorKey::Paste) {
        start_paste(editor, view_key);
    }
    if pressed(EditorKey::Duplicate) {
        duplicate_selection(editor, frame);
    }
    if pressed(EditorKey::SelectAll) {
        super::select::select_all(editor, frame);
    }
    if pressed(EditorKey::MakeBlueprint) {
        blueprint::make_blueprint_from_selection(editor, frame);
    }
    if let Some([x, y]) = EditorKey::Move.arrow(&input) {
        let step = if EditorKey::FineStep.held(&input) {
            1.0 / 16.0
        } else {
            1.0
        };
        if !editor.selection.parts.is_empty() {
            move_selection(editor, frame, x as f32 * step, y as f32 * step);
        } else {
            let view = editor.views.area.view_mut(view_key);
            let cursor = view.cursor_cell.unwrap_or_else(|| view.center.cell());
            view.cursor_cell = Some(cursor.step((x, y)));
        }
    }
    if pressed(EditorKey::SelectAtCursor)
        && let Some(cursor) = editor.views.area.view(view_key).cursor_cell
    {
        let tile = cursor.center();
        if let Some(hit) = frame.hit(tile) {
            nodes::select(editor, &frame.bundle, vec![hit]);
        }
    }
    if pressed(EditorKey::ContextMenu) {
        let view = editor.views.area.view_mut(view_key);
        let at = view
            .cursor_cell
            .map(|c| frame.canvas.to_screen(c.center()))
            .unwrap_or(frame.canvas.screen.center());
        let tile = view.cursor_cell.map(|c| c.center()).unwrap_or(view.center);
        super::modules::open_menu(ui.ctx(), view, view_key, at, tile);
    }
}

/// Says that the cell under the brush shows what a placement paints, so a stroke on the bundle's
/// own cells there won't show.
fn under_placement_tip(frame: &AreaFrame, placement: &str, armed: &Armed) {
    let Armed::Layer { name, .. } = armed else {
        return;
    };
    let label = frame
        .module(placement)
        .map(|module| shapes::label(module, frame.reader.as_ref()))
        .unwrap_or_default();
    if let Some(bounds) = frame.module_bounds(placement) {
        paint::dashed_rect(
            &frame.response.ctx.layer_painter(frame.response.layer_id),
            frame.canvas.rect(bounds),
            EStroke::new(1.0, theme::DROP),
            4.0,
            3.0,
        );
    }
    frame.response.clone().on_hover_text_at_pointer(format!(
        "This cell shows {label}'s {name}: the stroke paints the {name} beneath it. \
         Double-click {label} in the scene to edit it in place instead."
    ));
}
