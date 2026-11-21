//! The viewport's tools and the hand on the camera: which tool is taken, how the pointer and the
//! keys drive it, and what each draws over the picture while it works.

use engine::core::math::{Offset, Rect};
use engine::core::units::{MeterSize, Meters};
use std::time::{Duration, Instant};

use engine::systems::area::TerrainDef;
use engine::systems::area::{AreaDef, AreaModule, AreaModuleKind};
use engine::systems::water::WaterDef;
use strum::VariantArray;
use ui::egui;
use ui::egui::{Color32, Pos2, Stroke as EStroke, Ui, Vec2};
use ui::paint;
use ui::theme;
use ui::words;

use super::brush::{SculptMode, Stroke, StrokeKind};
use super::camera::{self, OrbitCamera};
use super::modules::{context_menu_at, open_add_menu, show_add_menu};
use super::select::{
    copy_selection, delete_selection, duplicate_selection, move_selection, paste_anchor,
    paste_in_place, select_input, selected_nodes, snapped_delta, start_paste,
};
use super::water::{Flood, FloodRefused, OVER_THE_GROUND, WaterMode};
use super::{
    AreaDrag, AreaFrame, AreaTool, Placeable, blueprint, nodes, pins, place, placement, scatter,
    shapes, stamp,
};
use crate::app::{Editor, PlacePick};
use crate::clipboard::Clipboard;
use crate::keymap::EditorKey;
use crate::picker::Dragged;
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
        editor.bottom_tab = bottom;
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
        AreaTool::Water => {
            for y in [-4.0, 1.0, 6.0] {
                painter.add(egui::Shape::line(
                    vec![
                        p(-8.0, y),
                        p(-4.0, y - 2.5),
                        p(0.0, y),
                        p(4.0, y - 2.5),
                        p(8.0, y),
                    ],
                    stroke,
                ));
            }
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
    level_by_wheel(editor, ui, view_key, frame);
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
    if let Some(ground) = frame.pointer_ground
        && let Some(Dragged::Row { table, key }) = response
            .dnd_release_payload::<Dragged>()
            .map(|dragged| (*dragged).clone())
        && super::spawner::spawns_table(&table)
    {
        super::spawner::drop_file(editor, frame, &key, ground);
        return;
    }
    let tool = editor.views.area.view(view_key).tool;
    if response.secondary_clicked() {
        context_menu_at(editor, view_key, frame);
    }
    match tool {
        AreaTool::Select => select_input(editor, ui, view_key, frame, modifiers),
        AreaTool::Sculpt | AreaTool::Paint => brush_input(editor, ui, view_key, frame, tool),
        AreaTool::Water => water_input(editor, ui, view_key, frame),
        AreaTool::Place => place::input(editor, ui, view_key, frame),
        AreaTool::Scatter => scatter::input(editor, ui, view_key, frame),
        AreaTool::Module => {
            if response.clicked()
                && let Some(ground) = frame.pointer_ground
            {
                let view = editor.views.area.view_mut(view_key);
                match view.placing.take() {
                    Some(kind) => {
                        view.set_tool(AreaTool::Select);
                        super::modules::add_module(editor, view_key, frame, kind, ground);
                    }
                    None => open_add_menu(ui, editor, view_key, frame, ground),
                }
            }
        }
    }
    show_add_menu(ui, editor, view_key, frame);
}

pub(crate) fn navigate(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    frame: &AreaFrame,
) -> bool {
    let farthest = OrbitCamera::framing_distance(frame.canvas.area().size.bounds());
    let view = editor.views.area.view_mut(view_key);
    camera::navigate(
        ui,
        frame.response,
        &mut view.camera,
        frame.canvas.screen.height(),
        farthest,
    )
}

/// How long a frame lets a flood spread for.
const FLOOD_A_FRAME: Duration = Duration::from_millis(6);
/// How far a notch of the wheel moves a flood's level, and how many points the wheel scrolls a
/// notch.
const LEVEL_A_NOTCH: Meters = Meters(0.25);
const POINTS_A_NOTCH: f32 = 50.0;

/// While the Water tool shows a flood, the wheel moves its level in place of the camera, which
/// the zoom key still gives the wheel to.
fn level_by_wheel(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view_mut(view_key);
    if view.tool != AreaTool::Water
        || view.water.mode != WaterMode::Flood
        || view.water.laying
        || view.water.flood.is_none()
        || !frame.response.hovered()
        || ui.input(|i| EditorKey::Zoom.held(i))
    {
        return;
    }
    let scrolled = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta).y);
    if scrolled == 0.0 {
        return;
    }
    let from = view
        .water
        .level
        .or(view.water.flood.as_ref().map(|flood| flood.level));
    if let Some(from) = from {
        view.water.level = Some(Meters(from.0 + scrolled / POINTS_A_NOTCH * LEVEL_A_NOTCH.0));
    }
}

/// The Water tool at work: a level picked off water that lies, the flood shown from under the
/// pointer and laid at a click, or a stroke of the brush over water.
fn water_input(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let response = frame.response;
    let Some(nodes::SceneBundle::Area(area)) = nodes::bundle_of(editor, &frame.bundle) else {
        if response.clicked() || response.drag_started() {
            editor.toast("A blueprint has no ground to lay water on; lay it in an area");
        }
        return;
    };
    let view = editor.views.area.view_mut(view_key);
    let picking = view.water.picking_level || ui.input(|i| EditorKey::Unsnapped.held(i));
    if picking && response.clicked() {
        let Some(ground) = frame.pointer_ground else {
            return;
        };
        if area.water.depth_at(&area.heights, ground).0 > 0.0 {
            let lying = area
                .waters
                .get(usize::from(area.water_map.at(ground)))
                .and_then(|&water| frame.reader.path_of(water));
            view.water.cancel();
            view.water.level = Some(area.water.height_at(ground));
            if let Some(lying) = lying {
                view.water.file = Some(lying.to_owned());
            }
        } else {
            editor.toast("No water lies there to take a level from");
        }
        return;
    }
    if view.water.picking_level {
        return;
    }
    match view.water.mode {
        WaterMode::Flood => flood_input(editor, ui, view_key, frame, &area),
        _ => brush_input(editor, ui, view_key, frame, AreaTool::Water),
    }
}

/// A flood: shown from the ground under the pointer as far as it has spread, a little farther
/// every frame, and laid as one step by a click once it stands whole.
fn flood_input(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: &str,
    frame: &AreaFrame,
    area: &AreaDef,
) {
    let response = frame.response;
    let view = editor.views.area.view_mut(view_key);
    let picked = view
        .water
        .file
        .as_deref()
        .and_then(|path| frame.reader.id_of::<WaterDef>(path));
    let Some(water) = picked else {
        view.water.cancel();
        if response.clicked() {
            editor.toast("Pick a water to lay in the inspector, or make a new one there");
        }
        return;
    };
    let tool = &mut view.water;
    if !tool.laying {
        let at = frame
            .pointer_ground
            .or(tool.flood.as_ref().map(|flood| flood.from));
        if let Some(at) = at {
            let stands = Meters(area.heights.height_at(at).0.max(area.water.height_at(at).0));
            let shown = tool.flood.as_ref().is_some_and(|flood| {
                flood.water == water
                    && tool.level.is_none_or(|level| level == flood.level)
                    && (frame.pointer_ground.is_none() || flood.floods(at))
            });
            if !shown {
                let level = tool.level.unwrap_or(stands + OVER_THE_GROUND);
                match Flood::begin(area, at, water, level) {
                    Ok(flood) => {
                        tool.flood = Some(flood);
                        tool.refused = None;
                    }
                    Err(refused) => {
                        tool.flood = None;
                        tool.refused = Some(refused);
                    }
                }
            }
        }
        if response.clicked() && tool.flood.is_some() {
            tool.laying = true;
        }
    }
    let Some(flood) = &mut tool.flood else {
        return;
    };
    if !flood.is_whole() {
        let until = Instant::now() + FLOOD_A_FRAME;
        flood.spread(|| Instant::now() < until);
        ui.ctx().request_repaint();
    }
    if tool.laying && flood.is_whole() {
        tool.laying = false;
        if let Some(flood) = tool.flood.take() {
            super::brush::commit(editor, &frame.bundle, &flood.stroke);
        }
    }
}

/// A stroke of the brush: begun where the button goes down on the ground, applied every frame
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
        && let Some(ground) = frame.drag_ground()
    {
        let what = match tool {
            AreaTool::Paint => {
                let picked = view
                    .paint_terrain
                    .as_deref()
                    .and_then(|path| frame.reader.id_of::<TerrainDef>(path));
                let Some(terrain) = picked else {
                    editor.toast("Pick a terrain to paint with in the inspector");
                    return;
                };
                StrokeKind::Paint {
                    terrain,
                    mask: view.paint_mask,
                }
            }
            AreaTool::Water => {
                let picked = view
                    .water
                    .file
                    .as_deref()
                    .and_then(|path| frame.reader.id_of::<WaterDef>(path));
                let Some(water) = picked else {
                    editor.toast("Pick a water to lay in the inspector, or make a new one there");
                    return;
                };
                StrokeKind::Water {
                    water,
                    mode: view.water.mode,
                }
            }
            _ => StrokeKind::Sculpt(sculpt_mode(view.sculpt, modifiers)),
        };
        let Some(mut stroke) = Stroke::begin(&area, ground, what) else {
            editor.toast(format!(
                "{} already names every terrain or water it can; pick one it has",
                frame.bundle
            ));
            return;
        };
        stroke.apply(&view.brush, ground, seconds.max(1.0 / 60.0), view.level);
        view.stroke = Some(stroke);
    } else if response.dragged_by(egui::PointerButton::Primary)
        && let Some(ground) = frame.drag_ground()
        && let Some(stroke) = &mut view.stroke
    {
        stroke.apply(&view.brush, ground, seconds, view.level);
    }
    if (response.drag_stopped_by(egui::PointerButton::Primary) || response.clicked())
        && let Some(stroke) = view.stroke.take()
    {
        super::brush::commit(editor, &frame.bundle, &stroke);
    }
}

/// The sculpt mode the modifiers turn the brush to: Shift inverts raising and lowering, Ctrl
/// smooths.
fn sculpt_mode(chosen: SculptMode, modifiers: egui::Modifiers) -> SculptMode {
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
    let Some(ground) = frame.pointer_ground else {
        return;
    };
    let picked = pins::PickedPlace::at(&frame.content, area_id, ground, fine);
    let (at, color) = (picked.at, picked.color());
    painter.circle_stroke(frame.canvas.to_screen(at), 6.0, EStroke::new(2.0, color));
    let what = match &picked.verdict {
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
        picked.commit(editor, &frame.bundle);
    }
}

pub fn overlay(editor: &mut Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    let view = editor.views.area.view(view_key);
    let accent = Color32::from_rgb(255, 230, 120);
    match view.drag.as_ref() {
        Some(AreaDrag::Box { from }) => {
            if let Some(ground) = frame.drag_ground() {
                painter.add(egui::Shape::closed_line(
                    canvas.outline(Rect::from_points([*from, ground])),
                    EStroke::new(1.0, accent),
                ));
            }
        }
        Some(AreaDrag::Move { from, moved: true }) => {
            if let Some(ground) = frame.drag_ground() {
                let alt = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
                let (dx, dy) = snapped_delta(*from, ground, alt);
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
    fog_overlay(editor, painter, frame);
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
        && let Some(ground) = frame.pointer_ground
    {
        let at = paste_anchor(ground);
        let mut whole: Option<Rect<Meters>> = None;
        let ghosts = nodes::modules(&ghost.nodes);
        for module in &ghosts {
            let mut placed = module.module().clone();
            placed.shift(at.x, at.y);
            if let Some(bounds) = shapes::bounds(&placed, None) {
                let kind = placed.kind();
                let color = super::overlays::module_color(kind);
                painter.add(egui::Shape::dashed_line(
                    &canvas.outline(bounds),
                    EStroke::new(1.5, color),
                    6.0,
                    4.0,
                ));
                if matches!(
                    kind,
                    AreaModuleKind::Spawner | AreaModuleKind::Resident | AreaModuleKind::Object
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
            canvas.to_screen(ground) + Vec2::new(16.0, 8.0),
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
            let Some(ground) = frame.pointer_ground else {
                return;
            };
            let kind = view.placing.expect("a kind being placed");
            let color = super::overlays::module_color(kind);
            let bounds = super::modules::template_bounds(kind, crate::views::snap(ground, false));
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
            if let Some(ground) = frame.pointer_ground {
                painter.add(egui::Shape::closed_line(
                    canvas.circle(crate::views::snap(ground, false), 0.5),
                    EStroke::new(1.0, accent),
                ));
            }
        }
        AreaTool::Place => place::overlay(editor, painter, view_key, frame),
        AreaTool::Scatter => scatter::overlay(editor, painter, view_key, frame),
        AreaTool::Sculpt | AreaTool::Paint => brush_overlay(editor, painter, view_key, frame),
        AreaTool::Water => match view.water.mode {
            WaterMode::Flood => flood_overlay(editor, view_key, frame),
            _ => brush_overlay(editor, painter, view_key, frame),
        },
        AreaTool::Select => {}
    }
}

/// A fog bank selected alone, shown where its numbers apply: its sight as a ring on the ground
/// round the pointer, and its level top as the outline of the bank lifted to it.
fn fog_overlay(editor: &Editor, painter: &egui::Painter, frame: &AreaFrame) {
    let selected = selected_nodes(editor, frame);
    let [node] = selected.as_slice() else {
        return;
    };
    let Some(AreaModule::Fog(bank)) = frame.module(node) else {
        return;
    };
    let canvas = &frame.canvas;
    let ink = super::overlays::module_color(AreaModuleKind::Fog);
    let area = frame.area();
    let top = bank.clone().laid(&area.heights, &area.water).floor + bank.depth;
    let corners = engine::core::geometry::corners(bank.rect)
        .map(|corner| canvas.point(engine::core::render::world_point(corner, top)));
    painter.add(egui::Shape::dashed_line(
        &[corners.as_slice(), &corners[..1]].concat(),
        EStroke::new(1.0, ink),
        6.0,
        4.0,
    ));
    frame.labels.chip(
        corners[0],
        format!("top {} m", crate::forms::metres(top.0)),
        ink,
    );
    if let Some(pointer) = frame.pointer_ground {
        painter.add(egui::Shape::closed_line(
            canvas.circle(pointer, bank.sight.0),
            EStroke::new(1.5, ink),
        ));
        frame.labels.chip(
            canvas.to_screen(pointer) + Vec2::new(12.0, 10.0),
            format!("sight {} m", crate::forms::metres(bank.sight.0)),
            ink,
        );
    }
}

/// What a flood would lay, or why it lays nothing, said at the pointer.
fn flood_overlay(editor: &Editor, view_key: &str, frame: &AreaFrame) {
    let tool = &editor.views.area.view(view_key).water;
    let Some(pointer) = frame.pointer_ground else {
        return;
    };
    let ink = Color32::from_rgb(120, 190, 255);
    let said = if tool.picking_level {
        "click water to take its level".to_owned()
    } else {
        match (&tool.flood, tool.refused) {
            (Some(flood), _) => flood_said(flood),
            (None, Some(refused)) => refusal_said(refused, tool.level),
            (None, None) => return,
        }
    };
    frame.labels.chip(
        frame.canvas.to_screen(pointer) + Vec2::new(12.0, -8.0),
        said,
        ink,
    );
}

/// How much a flood lays and how deep, as far as it has spread.
pub(super) fn flood_said(flood: &Flood) -> String {
    format!(
        "{} · deepest {} m{}",
        area_said(flood.flooded),
        crate::forms::metres(flood.deepest.0),
        if flood.is_whole() {
            ""
        } else {
            " · spreading"
        }
    )
}

/// Why a flood lays nothing where it would start, against the level `set` or the one it took.
pub(super) fn refusal_said(refused: FloodRefused, set: Option<Meters>) -> String {
    let level = |stands: Meters| match set {
        Some(level) => format!("the level of {} m", crate::forms::metres(level.0)),
        None => format!("{} m", crate::forms::metres(stands.0 + OVER_THE_GROUND.0)),
    };
    match refused {
        FloodRefused::AboveTheLevel { ground } => format!(
            "The ground here stands at {} m, above {}.",
            crate::forms::metres(ground.0),
            level(ground)
        ),
        FloodRefused::AlreadyUnder { surface } => format!(
            "Water already stands here at {} m, above {}.",
            crate::forms::metres(surface.0),
            level(surface)
        ),
        FloodRefused::NoRoomForAWater => {
            "This area already names every water it can; pick one it has.".to_owned()
        }
    }
}

/// A stretch of ground of `square_metres` as a designer reads it: in hectares once it is one
/// tenth of one.
fn area_said(square_metres: f32) -> String {
    if square_metres >= 1000.0 {
        format!("{:.1} ha", square_metres / 10_000.0)
    } else {
        format!("{} m²", square_metres.round())
    }
}

/// The brush as a ring on the ground under the pointer, and a fainter ring where its strength
/// starts to fall off.
fn brush_overlay(editor: &Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    let Some(ground) = frame.pointer_ground else {
        return;
    };
    let canvas = &frame.canvas;
    let (color, what) = match view.tool {
        AreaTool::Paint => {
            let picked = view
                .paint_terrain
                .as_deref()
                .and_then(|path| editor.read::<TerrainDef>(path))
                .and_then(Result::ok);
            match picked {
                Some(terrain) => {
                    let [r, g, b, _] = <[u8; 4]>::from(terrain.color);
                    (Color32::from_rgb(r, g, b), terrain.name.clone())
                }
                None => (Color32::GRAY, "no terrain picked".to_owned()),
            }
        }
        AreaTool::Water => (
            Color32::from_rgb(120, 190, 255),
            view.water.mode.as_ref().to_owned(),
        ),
        _ => {
            let modifiers = painter.ctx().input(|i| i.modifiers);
            let mode = sculpt_mode(view.sculpt, modifiers);
            (Color32::from_rgb(255, 230, 120), mode.as_ref().to_owned())
        }
    };
    painter.add(egui::Shape::closed_line(
        canvas.circle(ground, view.brush.radius.0),
        EStroke::new(2.0, color),
    ));
    if view.brush.falloff > 0.0 {
        painter.add(egui::Shape::closed_line(
            canvas.circle(
                ground,
                view.brush.radius.0 * (1.0 - view.brush.falloff).max(0.05),
            ),
            EStroke::new(1.0, ui::ink::wash(color, 0.5)),
        ));
    }
    frame.labels.chip(
        canvas.to_screen(ground) + Vec2::new(12.0, -8.0),
        format!(
            "{what} · {} m · {}",
            crate::forms::metres(view.brush.radius.0),
            match view.tool {
                AreaTool::Paint => format!("{}% fray", (view.brush.falloff * 100.0).round()),
                AreaTool::Water => format!("{}% soft", (view.brush.falloff * 100.0).round()),
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
            view.toggle(super::AreaOverlay::Navmesh);
        }
        if pressed(EditorKey::ContoursOverlay) {
            view.toggle(super::AreaOverlay::Contours);
        }
        if pressed(EditorKey::FootprintsOverlay) {
            view.toggle(super::AreaOverlay::Footprints);
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
                            shapes::turn(&mut scene.module_mut(node)?.module, clockwise);
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
        let ground = frame
            .pointer_ground
            .unwrap_or(editor.views.area.view(view_key).camera.target);
        let view = editor.views.area.view_mut(view_key);
        super::modules::open_menu(ui.ctx(), view, view_key, at, ground);
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
