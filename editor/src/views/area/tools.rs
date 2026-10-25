//! The viewport's tools and the hand on the camera: which tool is taken, how the pointer and the
//! keys drive it, and what each draws over the picture while it works.

use engine::core::math::{Offset, Rect};
use engine::core::units::{Meters, TileSize};
use engine::systems::area::{AreaModule, AreaModuleKind};
use strum::VariantArray;
use ui::egui;
use ui::egui::{Color32, Pos2, Rect as ERect, Stroke as EStroke, Ui, Vec2};
use ui::paint;
use ui::theme;
use ui::words;

use super::brush::{SculptMode, Stroke, StrokeKind};
use super::camera::OrbitCamera;
use super::modules::{context_menu_at, open_add_menu, show_add_menu};
use super::select::{
    copy_selection, delete_selection, duplicate_selection, move_selection, paste_anchor,
    paste_in_place, select_input, selected_nodes, snapped_delta, start_paste,
};
use super::{
    AreaDrag, AreaFrame, AreaTool, Placeable, blueprint, nodes, pins, place, placement, scatter,
    shapes, stamp,
};
use crate::app::{Clipboard, Editor, PlacePick};
use crate::keymap::EditorKey;
use crate::tabs::file_title;

/// Metres a second the arrow keys pan at, scaled by how far the camera stands back.
const PAN_PER_SECOND: f32 = 0.6;

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
    let stroke = EStroke::new(1.8, ink);
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
        AreaTool::Sculpt => {
            painter.add(egui::Shape::line(
                vec![
                    p(-8.0, 6.0),
                    p(-5.0, 2.0),
                    p(-2.0, -5.0),
                    p(1.0, -2.0),
                    p(4.0, -6.0),
                    p(8.0, 6.0),
                ],
                stroke,
            ));
            painter.line_segment([p(-8.0, 6.0), p(8.0, 6.0)], stroke);
        }
        AreaTool::Paint => {
            painter.line_segment([p(5.0, -7.0), p(-2.0, 0.0)], EStroke::new(2.5, ink));
            painter.add(egui::Shape::convex_polygon(
                vec![p(-3.0, -1.0), p(0.0, 2.0), p(-3.0, 7.0), p(-7.0, 5.0)],
                ink,
                EStroke::NONE,
            ));
        }
        AreaTool::Place => {
            painter.circle_stroke(p(0.0, -3.0), 4.5, stroke);
            painter.line_segment([p(0.0, 1.5), p(0.0, 7.0)], stroke);
            painter.line_segment([p(-5.0, 7.0), p(5.0, 7.0)], stroke);
        }
        AreaTool::Scatter => {
            for (x, y) in [
                (-5.0, -4.0),
                (3.0, -6.0),
                (-1.0, 1.0),
                (5.0, 3.0),
                (-6.0, 5.0),
            ] {
                painter.circle_filled(p(x, y), 2.0, ink);
            }
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
    if navigate(editor, ui, view_key, frame) {
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
        AreaTool::Sculpt | AreaTool::Paint => brush_input(editor, ui, view_key, frame, tool),
        AreaTool::Place => place::input(editor, ui, view_key, frame),
        AreaTool::Scatter => scatter::input(editor, ui, view_key, frame),
        AreaTool::Module => {
            if response.clicked()
                && let Some(tile) = frame.pointer_tile
            {
                let view = editor.views.area.view_mut(view_key);
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

/// Moves the camera as the pointer asks: the right button orbits, the middle button or Space with
/// the left drags the ground along, the wheel draws back and in. Answers whether the pointer is
/// busy with the camera.
fn navigate(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) -> bool {
    let response = frame.response;
    let view = editor.views.area.view_mut(view_key);
    let height = frame.canvas.screen.height();
    if response.dragged_by(egui::PointerButton::Secondary) {
        let delta = response.drag_delta();
        view.camera.orbit(bevy::math::Vec2::new(delta.x, delta.y));
        return true;
    }
    let space = ui.input(|i| EditorKey::Pan.held(i));
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || (space && response.dragged_by(egui::PointerButton::Primary));
    if panning {
        let delta = response.drag_delta();
        let per_pixel = view.camera.metres_per_pixel(height);
        view.camera
            .pan(bevy::prelude::Vec2::new(-delta.x, delta.y) * per_pixel);
        return true;
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        let pinch = ui.input(|i| i.zoom_delta());
        if pinch != 1.0 {
            view.camera.zoom(1.0 / pinch);
        } else if scroll.y != 0.0 {
            view.camera.zoom((-scroll.y / 240.0).exp2());
        }
    }
    false
}

/// A sculpt or paint stroke: begun where the button goes down on the ground, applied every frame
/// the pointer is dragged, written as one step when it comes up.
fn brush_input(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    frame: &AreaFrame,
    tool: AreaTool,
) {
    let response = frame.response;
    let Some(nodes::SceneBundle::Area(area)) = nodes::bundle_of(editor, &frame.bundle) else {
        if response.clicked() || response.drag_started() {
            editor.toast("A blueprint has no terrain to shape; sculpt and paint in an area");
        }
        return;
    };
    let modifiers = ui.input(|i| i.modifiers);
    let seconds = ui.input(|i| i.stable_dt).min(0.1);
    let view = editor.views.area.view_mut(view_key);
    if (response.drag_started_by(egui::PointerButton::Primary) || response.clicked())
        && let Some(tile) = frame.drag_tile()
    {
        let what = match tool {
            AreaTool::Paint => StrokeKind::Paint {
                slot: view.paint_slot,
            },
            _ => StrokeKind::Sculpt(sculpt_mode(view.sculpt, modifiers)),
        };
        let mut stroke = Stroke::begin(&area, tile, what);
        stroke.apply(&view.brush, tile, seconds.max(1.0 / 60.0), view.level);
        view.stroke = Some(stroke);
    } else if response.dragged_by(egui::PointerButton::Primary)
        && let Some(tile) = frame.drag_tile()
        && let Some(stroke) = &mut view.stroke
    {
        stroke.apply(&view.brush, tile, seconds, view.level);
    }
    if (response.drag_stopped_by(egui::PointerButton::Primary) || response.clicked())
        && let Some(stroke) = view.stroke.take()
    {
        super::brush::commit(editor, &frame.bundle, &stroke);
    }
}

/// The sculpt mode the modifiers turn the brush to: Shift inverts raising and lowering, Ctrl
/// smooths.
pub fn sculpt_mode(chosen: SculptMode, modifiers: egui::Modifiers) -> SculptMode {
    if EditorKey::ToggleSelection.held_among(modifiers) {
        return SculptMode::Smooth;
    }
    if EditorKey::ExtendSelection.held_among(modifiers) {
        return match chosen {
            SculptMode::Raise => SculptMode::Lower,
            SculptMode::Lower => SculptMode::Raise,
            other => other,
        };
    }
    chosen
}

/// A place being picked for a file's field: the ground under the pointer, shown as the game would
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
    let canvas = &frame.canvas;
    let view = editor.views.area.view(view_key);
    let accent = Color32::from_rgb(255, 230, 120);
    match view.drag() {
        Some(AreaDrag::Box { from }) => {
            if let Some(tile) = frame.drag_tile() {
                painter.add(egui::Shape::closed_line(
                    canvas.outline(Rect::from_points([*from, tile])),
                    EStroke::new(1.0, accent),
                ));
            }
        }
        Some(AreaDrag::Move { from, moved: true }) => {
            if let Some(tile) = frame.drag_tile() {
                let alt = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
                let (dx, dy) = snapped_delta(*from, tile, alt);
                for node in selected_nodes(editor, frame) {
                    if let Some(bounds) = frame.module_bounds(&node) {
                        let moved = bounds.translate(Offset::new(dx, dy));
                        painter.add(egui::Shape::closed_line(
                            canvas.outline(moved),
                            EStroke::new(1.5, accent),
                        ));
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
                painter.add(egui::Shape::dashed_line(
                    &canvas.outline(bounds),
                    EStroke::new(1.5, color),
                    6.0,
                    4.0,
                ));
                if matches!(
                    kind,
                    AreaModuleKind::Population | AreaModuleKind::Resident | AreaModuleKind::Object
                ) {
                    frame.labels.chip(
                        canvas.rect(bounds).min + Vec2::new(0.0, -4.0),
                        shapes::label(&placed, frame.reader.as_ref()),
                        color,
                    );
                }
                whole = Some(whole.map_or(bounds, |w| w.union(&bounds)));
            }
        }
        if let Some(whole) = whole {
            painter.add(egui::Shape::dashed_line(
                &canvas.outline(whole),
                EStroke::new(1.0, Color32::from_rgba_unmultiplied(255, 120, 160, 160)),
                8.0,
                6.0,
            ));
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
    match view.tool {
        AreaTool::Module if view.placing.is_some() => {
            let Some(tile) = frame.pointer_tile else {
                return;
            };
            let kind = view.placing.expect("a kind being placed");
            let color = super::overlays::module_color(Some(kind));
            let bounds = super::modules::template_bounds(kind, super::select::snap_to(tile, false));
            painter.add(egui::Shape::dashed_line(
                &canvas.outline(bounds),
                EStroke::new(1.5, color),
                6.0,
                4.0,
            ));
            frame.labels.chip(
                canvas.rect(bounds).min + Vec2::new(0.0, -4.0),
                format!("new {}", kind.tag().replace('_', " ")),
                color,
            );
        }
        AreaTool::Module => {
            if let Some(tile) = frame.pointer_tile {
                painter.add(egui::Shape::closed_line(
                    canvas.circle(super::select::snap_to(tile, false), 0.5),
                    EStroke::new(1.0, accent),
                ));
            }
        }
        AreaTool::Place => place::overlay(editor, painter, view_key, frame),
        AreaTool::Scatter => scatter::overlay(editor, painter, view_key, frame),
        AreaTool::Sculpt | AreaTool::Paint => brush_overlay(editor, painter, view_key, frame),
        AreaTool::Select => {}
    }
}

/// The brush as a ring on the ground under the pointer, and a fainter ring where its strength
/// starts to fall off.
fn brush_overlay(editor: &Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    let Some(tile) = frame.pointer_tile else {
        return;
    };
    let canvas = &frame.canvas;
    let (color, what) = match view.tool {
        AreaTool::Paint => {
            let slot = view
                .paint_slot
                .min(frame.area().terrain.materials.len() - 1);
            let ground = frame.area().terrain.materials[slot].get(&frame.content);
            let [r, g, b, _] = <[u8; 4]>::from(ground.color);
            (Color32::from_rgb(r, g, b), ground.name.clone())
        }
        _ => {
            let modifiers = painter.ctx().input(|i| i.modifiers);
            let mode = sculpt_mode(view.sculpt, modifiers);
            (Color32::from_rgb(255, 230, 120), mode.as_ref().to_owned())
        }
    };
    painter.add(egui::Shape::closed_line(
        canvas.circle(tile, view.brush.radius.0),
        EStroke::new(2.0, color),
    ));
    if view.brush.falloff > 0.0 {
        painter.add(egui::Shape::closed_line(
            canvas.circle(
                tile,
                view.brush.radius.0 * (1.0 - view.brush.falloff).max(0.05),
            ),
            EStroke::new(1.0, ui::ink::wash(color, 0.5)),
        ));
    }
    frame.labels.chip(
        canvas.to_screen(tile) + Vec2::new(12.0, -8.0),
        format!(
            "{what} · {} m · {}",
            crate::forms::metres(view.brush.radius.0),
            match view.tool {
                AreaTool::Paint => format!("{}% strength", (view.brush.strength * 100.0).round()),
                _ => format!(
                    "{} inverts · {} smooths",
                    EditorKey::ExtendSelection.name(),
                    EditorKey::ToggleSelection.name()
                ),
            }
        ),
        color,
    );
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
        if pressed(EditorKey::NavmeshOverlay) {
            view.overlay_navmesh = !view.overlay_navmesh;
        }
        if pressed(EditorKey::ContoursOverlay) {
            view.overlay_contours = !view.overlay_contours;
        }
        if pressed(EditorKey::FootprintsOverlay) {
            view.overlay_footprints = !view.overlay_footprints;
        }
        if pressed(EditorKey::BrushSmaller) {
            view.brush.shrink();
        }
        if pressed(EditorKey::BrushLarger) {
            view.brush.grow();
        }
        if pressed(EditorKey::TopDown) {
            view.camera.top_down();
        }
    }
    if let Some(tool) = switched {
        took(editor, tool);
    }
    if pressed(EditorKey::FrameSelection) {
        frame_selection(editor, view_key, frame);
    }
    let clockwise = pressed(EditorKey::TurnClockwise);
    if clockwise || pressed(EditorKey::TurnCounter) {
        let view = editor.views.area.view(view_key);
        match (view.tool, &view.placeable) {
            (AreaTool::Place, Some(Placeable::Blueprint(_))) => {
                stamp::turn_stamp(editor, view_key, clockwise)
            }
            (AreaTool::Place, _) => place::turn(editor, view_key, frame, clockwise),
            _ => {
                let turned = selected_nodes(editor, frame);
                let props: Vec<String> = turned
                    .iter()
                    .filter(|node| matches!(frame.module(node), Some(AreaModule::Prop(_))))
                    .cloned()
                    .collect();
                if !props.is_empty() {
                    place::turn(editor, view_key, frame, clockwise);
                } else if !turned.is_empty() {
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
        if editor.selection.parts.is_empty() {
            let view = editor.views.area.view_mut(view_key);
            let step = view.camera.distance.0 * PAN_PER_SECOND * input.stable_dt.min(0.1) * 8.0;
            view.camera
                .pan(bevy::prelude::Vec2::new(x as f32, -y as f32) * step);
        } else {
            let step = if EditorKey::FineStep.held(&input) {
                1.0 / 16.0
            } else {
                0.5
            };
            move_selection(editor, frame, x as f32 * step, y as f32 * step);
        }
    }
    if pressed(EditorKey::ContextMenu) {
        let at = frame.pointer.unwrap_or(frame.canvas.screen.center());
        let tile = frame
            .pointer_tile
            .unwrap_or(editor.views.area.view(view_key).camera.target);
        let view = editor.views.area.view_mut(view_key);
        super::modules::open_menu(ui.ctx(), view, view_key, at, tile);
    }
}

/// Brings the selection into frame, or the whole area when nothing is selected.
fn frame_selection(editor: &mut Editor, view_key: &str, frame: &AreaFrame) {
    let selected = selected_nodes(editor, frame);
    let bounds = selected
        .iter()
        .filter_map(|node| frame.module_bounds(node))
        .reduce(|a, b| a.union(&b))
        .unwrap_or_else(|| frame.size().bounds());
    let view = editor.views.area.view_mut(view_key);
    view.camera.frame(bounds.inflate(1.0, 1.0));
}

/// Where the camera stands, for whoever asks after the view.
pub fn camera_of(editor: &Editor, view_key: &str) -> OrbitCamera {
    editor.views.area.view(view_key).camera
}

/// The screen box around a ground rectangle, as the overlays place labels by.
pub fn screen_box(frame: &AreaFrame, rect: Rect<Meters>) -> ERect {
    frame.canvas.rect(rect)
}
