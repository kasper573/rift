use engine::core::math::{Offset, Rect};
use engine::core::units::{Cell, Meters, TilePos};
use engine::systems::area::{AreaModule, AreaModuleKind};
use strum::VariantArray;
use ui::egui;
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Ui, Vec2};
use ui::paint;
use ui::theme;
use ui::words;

use super::modules::{context_menu_at, open_add_menu, show_add_menu};
use super::select::{
    copy_selection, delete_selection, duplicate_selection, move_selection, paste_anchor,
    paste_in_place, select_input, selected_nodes, snapped_delta, start_paste,
};
use super::{
    AreaDrag, AreaFrame, AreaTool, MAX_ZOOM, MIN_ZOOM, blueprint, nodes, pins, placement, shapes,
    stamp,
};
use crate::app::{Clipboard, Editor, PlacePick};
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
        took(editor, tool);
    }
}

/// What taking a tool brings along: its bottom tab.
fn took(editor: &mut Editor, tool: AreaTool) {
    if let Some(bottom) = tool.bottom_tab() {
        editor.panels.bottom = bottom;
    }
}

fn tool_icon(painter: &egui::Painter, tool: AreaTool, c: Pos2, ink: Color32) {
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
    let tool = editor.views.area.view(view_key).tool;
    if response.secondary_clicked() {
        context_menu_at(editor, view_key, frame);
    }
    match tool {
        AreaTool::Select => select_input(editor, ui, view_key, frame, modifiers),
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

/// A place being picked for a file's field: the cell under the pointer, shown as the game would
/// take it, and taken by a click.
fn pick_place(
    editor: &mut Editor,
    painter: &egui::Painter,
    frame: &AreaFrame,
    pick: &PlacePick,
    fine: bool,
) {
    let Some(area_id) = frame.area_id() else {
        return;
    };
    let Some(tile) = frame.pointer_tile else {
        return;
    };
    let at = super::select::snap_to(tile, fine);
    let place = engine::systems::script::Place { area: area_id, at };
    let verdict = engine::systems::area::check_place(&frame.content, place);
    let color = if verdict.is_ok() {
        Color32::from_rgb(255, 230, 120)
    } else {
        theme::DANGER
    };
    painter.circle_stroke(frame.canvas.to_screen(at), 6.0, EStroke::new(2.0, color));
    let what = match &verdict {
        Ok(()) => "click to pick".to_owned(),
        Err(problem) => problem.expected.clone(),
    };
    frame.labels.chip(
        frame.canvas.to_screen(at) + Vec2::new(10.0, -6.0),
        format!(
            "{}, {} · {what}",
            crate::forms::metres(at.x),
            crate::forms::metres(at.y)
        ),
        color,
    );
    editor.status.left.push(format!(
        "picking a place for {} · {} cancels",
        file_title(&pick.file),
        EditorKey::Cancel.name()
    ));
    if frame.response.clicked() {
        if let Err(problem) = verdict {
            editor.toast(format!("{}: {}", problem.rule, problem.expected));
            return;
        }
        pins::commit_pick(editor, &frame.bundle, at);
    }
}

pub fn overlay(editor: &mut Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let canvas = frame.canvas;
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
            "{} pastes at the same spot",
            EditorKey::PasteInPlace.name()
        ));
    }
    let view = editor.views.area.view(view_key);
    let Some(cell) = frame.pointer_cell() else {
        return;
    };
    match view.tool {
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
        AreaTool::Module => {
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
    }
    if let Some(tool) = switched {
        took(editor, tool);
    }
    let clockwise = pressed(EditorKey::TurnClockwise);
    if clockwise || pressed(EditorKey::TurnCounter) {
        match editor.views.area.view(view_key).tool {
            AreaTool::Stamp => stamp::turn_stamp(editor, view_key, clockwise),
            _ => {
                let turned = selected_nodes(editor, frame);
                if !turned.is_empty() {
                    nodes::change_scene(editor, &frame.bundle, |scene| {
                        for node in &turned {
                            shapes::turn(
                                &mut nodes::module_node_mut(scene, node)?.module,
                                clockwise,
                            );
                        }
                        Some(format!("Turn {} modules", turned.len()))
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
