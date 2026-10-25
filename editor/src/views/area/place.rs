//! The Place tool: a prop from the palette follows the pointer over the terrain as a ghost, snapped
//! to the half metre and turned as asked, and a click stands one where it is; a blueprint from the
//! Blueprints tab is stamped as before.

use engine::core::geometry::Degrees;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaModule;
use engine::systems::area::prop::{PropDef, PropPlacement};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::scenery::prop_transform;
use engine::systems::movement::AGENT_RADIUS;
use ui::egui;
use ui::egui::{Stroke as EStroke, Ui, Vec2};
use ui::theme;

use super::viewport::Ghost;
use super::{AreaFrame, AreaSnapshot, AreaTool, Placeable, modules, select, stamp};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;

/// How far one press of the turn key turns a prop.
pub const TURN_STEP: Degrees = Degrees(15.0);

/// Snaps a ground point to the half metre, or the sixteenth when `fine`.
pub fn snap(at: Pos<Meters>, fine: bool) -> Pos<Meters> {
    if fine {
        select::snap_to(at, true)
    } else {
        Pos::new((at.x * 2.0).round() / 2.0, (at.y * 2.0).round() / 2.0)
    }
}

pub fn input(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    match view.placeable.clone() {
        Some(Placeable::Blueprint(_)) => stamp::stamp_input(editor, ui, view_key, frame),
        Some(Placeable::Prop(prop)) => {
            let fine = ui.input(|i| EditorKey::Unsnapped.held(i));
            if frame.response.clicked()
                && let Some(tile) = frame.drag_tile()
            {
                let yaw = editor.views.area.view(view_key).place_yaw;
                place_prop(editor, frame, &prop, snap(tile, fine), yaw);
            }
        }
        None => {}
    }
}

/// Stands a prop at `at` in the bundle's insertion folder.
pub fn place_prop(
    editor: &mut Editor,
    frame: &AreaSnapshot,
    prop: &str,
    at: Pos<Meters>,
    yaw: Degrees,
) {
    let Some(id) = frame.reader.id_of::<PropDef>(prop) else {
        editor.toast(format!("{prop} is not a prop that loads"));
        return;
    };
    let module = AreaModule::Prop(PropPlacement {
        prop: id,
        at,
        yaw,
        scale: 1.0,
    });
    let label = format!(
        "Place {} in {}",
        file_title(prop),
        file_title(&frame.bundle)
    );
    let bundle = frame.bundle.clone();
    let outcome = modules::place_modules(
        editor,
        &bundle,
        vec![SceneNode::Module(ModuleNode::new(module))],
        &label,
    );
    if let modules::PlaceOutcome::Partly { reason, .. } = outcome {
        editor.toast(reason);
    }
}

/// Turns what the Place tool puts down next, or the selected props, by a step.
pub fn turn(editor: &mut Editor, view_key: &str, frame: &AreaFrame, clockwise: bool) {
    let step = Degrees(if clockwise { TURN_STEP.0 } else { -TURN_STEP.0 });
    let view = editor.views.area.view_mut(view_key);
    view.place_yaw = Degrees((view.place_yaw.0 + step.0).rem_euclid(360.0));
    let selected = select::selected_nodes(editor, frame);
    let turned: Vec<String> = selected
        .into_iter()
        .filter(|node| matches!(frame.module(node), Some(AreaModule::Prop(_))))
        .collect();
    if !turned.is_empty() {
        super::nodes::change_scene(editor, &frame.bundle, |scene| {
            for node in &turned {
                if let Some(ModuleNode {
                    module: AreaModule::Prop(prop),
                    ..
                }) = super::nodes::module_node_mut(scene, node)
                {
                    prop.yaw = Degrees((prop.yaw.0 + step.0).rem_euclid(360.0));
                }
            }
            Some(format!("Turn {} props", turned.len()))
        });
    }
}

/// The prop following the pointer, for the viewport to stand where it would land.
pub fn ghost(editor: &Editor, view_key: &str, frame: &AreaFrame) -> Option<Ghost> {
    let view = editor.views.area.view(view_key);
    if view.tool != AreaTool::Place {
        return None;
    }
    let prop = view.prop()?;
    let tile = frame.pointer_tile?;
    let fine = frame.response.ctx.input(|i| EditorKey::Unsnapped.held(i));
    let id = frame.reader.id_of::<PropDef>(prop)?;
    let def = frame.content.by_path::<PropDef>(prop)?.get(&frame.content);
    let placed = PropPlacement {
        prop: id,
        at: snap(tile, fine),
        yaw: view.place_yaw,
        scale: 1.0,
    };
    Some(Ghost {
        model: def.model.path(),
        transform: prop_transform(&frame.area().heights, &placed),
    })
}

/// The footprint of the ghost and the outline the navmesh keeps clear of it, drawn where it would
/// stand.
pub fn overlay(editor: &Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    match &view.placeable {
        Some(Placeable::Blueprint(_)) => stamp::stamp_overlay(editor, painter, view_key, frame),
        Some(Placeable::Prop(prop)) => {
            let Some(tile) = frame.pointer_tile else {
                return;
            };
            let fine = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
            let at = snap(tile, fine);
            let color =
                super::overlays::module_color(Some(engine::systems::area::AreaModuleKind::Prop));
            if let Some(def) = frame
                .content
                .by_path::<PropDef>(prop)
                .map(|id| id.get(&frame.content))
            {
                draw_footprint(
                    painter,
                    frame,
                    def,
                    at,
                    view.place_yaw,
                    1.0,
                    EStroke::new(1.5, color),
                );
            }
            frame.labels.chip(
                frame.canvas.to_screen(at) + Vec2::new(10.0, -6.0),
                format!(
                    "{} · {}° · {} turns",
                    file_title(prop),
                    view.place_yaw.0.round(),
                    EditorKey::TurnClockwise.name()
                ),
                color,
            );
        }
        None => {
            frame.labels.chip(
                frame.canvas.screen.min + Vec2::new(60.0, 40.0),
                "pick a prop in the Palette or a blueprint in Blueprints to place",
                theme::MUTED,
            );
        }
    }
}

/// A prop's footprint on the ground and, outside it, the outline the navmesh keeps walkers clear
/// of; a stake where the prop takes no ground.
pub fn draw_footprint(
    painter: &egui::Painter,
    frame: &AreaFrame,
    def: &PropDef,
    at: Pos<Meters>,
    yaw: Degrees,
    scale: f32,
    stroke: EStroke,
) {
    let canvas = &frame.canvas;
    let outline = def.footprint.outline(at, yaw, scale);
    if outline.len() >= 3 {
        painter.add(egui::Shape::closed_line(canvas.closed(&outline), stroke));
        let grown = grown(&outline, AGENT_RADIUS.0);
        painter.add(egui::Shape::dashed_line(
            &canvas.closed(&grown),
            EStroke::new(1.0, ui::ink::wash(stroke.color, 0.5)),
            4.0,
            4.0,
        ));
    } else {
        painter.add(egui::Shape::closed_line(canvas.circle(at, 0.25), stroke));
    }
    let heading = at + yaw.rotate(engine::core::math::Offset::new(0.0, -0.5));
    painter.add(egui::Shape::line(canvas.polyline(&[at, heading]), stroke));
}

/// The polygon `outline` pushed out by `by` metres: each corner moved out along the bisector of
/// its two edges, as far as a mitre there needs.
pub fn grown(outline: &[Pos<Meters>], by: f32) -> Vec<Pos<Meters>> {
    let count = outline.len();
    if count < 3 {
        return outline.to_vec();
    }
    let signed_area: f32 = (0..count)
        .map(|i| {
            let (a, b) = (outline[i], outline[(i + 1) % count]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let winding = if signed_area >= 0.0 { 1.0 } else { -1.0 };
    let normal = |from: Pos<Meters>, to: Pos<Meters>| {
        let edge = to - from;
        let length = edge.length().max(f32::EPSILON);
        engine::core::math::Offset::new(edge.y, -edge.x) * (winding / length)
    };
    (0..count)
        .map(|i| {
            let previous = outline[(i + count - 1) % count];
            let here = outline[i];
            let next = outline[(i + 1) % count];
            let (before, after) = (normal(previous, here), normal(here, next));
            let along = (1.0 + before.dot(after)).max(0.2);
            here + (before + after) * (by / along)
        })
        .collect()
}
