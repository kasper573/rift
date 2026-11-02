//! The Place tool: a prop from the palette follows the pointer over the terrain as a ghost, snapped
//! to the half metre and turned as asked, and a click stands one where it is; a blueprint from the
//! Blueprints tab is stamped where a click puts it.

use engine::core::math::Pos;
use engine::core::units::{Degrees, Meters};
use engine::systems::area::AreaModule;
use engine::systems::area::prop::{PropDef, PropPlacement};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::scenery::prop_transform;
use ui::egui;
use ui::egui::{Stroke as EStroke, Ui, Vec2};
use ui::theme;

use super::viewport::Ghost;
use super::{AreaFrame, AreaSnapshot, AreaTool, Placeable, modules, select, shapes, stamp};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;
use crate::views::snap;

/// How far one press of the turn key turns a prop.
const TURN_STEP: Degrees = Degrees(15.0);

pub fn input(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    match view.placeable.clone() {
        Some(Placeable::Blueprint(_)) => stamp::stamp_input(editor, ui, view_key, frame),
        Some(Placeable::Prop(prop)) => {
            let fine = ui.input(|i| EditorKey::Unsnapped.held(i));
            if frame.response.clicked()
                && let Some(ground) = frame.drag_ground()
            {
                let yaw = editor.views.area.view(view_key).place_yaw;
                place_prop(editor, frame, &prop, snap(ground, fine), yaw);
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
        sunk: Meters::default(),
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
    let ground = frame.pointer_ground?;
    let fine = frame.response.ctx.input(|i| EditorKey::Unsnapped.held(i));
    let id = frame.reader.id_of::<PropDef>(prop)?;
    let def = frame.content.by_path::<PropDef>(prop)?.get(&frame.content);
    let placed = PropPlacement {
        prop: id,
        at: snap(ground, fine),
        yaw: view.place_yaw,
        scale: 1.0,
        sunk: Meters::default(),
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
            let Some(ground) = frame.pointer_ground else {
                return;
            };
            let fine = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
            let at = snap(ground, fine);
            let color = super::overlays::module_color(engine::systems::area::AreaModuleKind::Prop);
            if let Some(def) = frame
                .content
                .by_path::<PropDef>(prop)
                .map(|id| id.get(&frame.content))
            {
                shapes::draw_footprint(
                    painter,
                    &frame.canvas,
                    &def.footprint,
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
