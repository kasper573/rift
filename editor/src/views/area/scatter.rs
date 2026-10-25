//! The Scatter tool: a rectangle dragged over the ground becomes a scatter of the palette's prop,
//! strewn at load from a seed of its own; the inspector reseeds it.

use engine::core::math::{Rect, Rng};
use engine::systems::area::AreaModule;
use engine::systems::area::prop::{PropDef, ScaleRange, Scatter};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use ui::egui;
use ui::egui::{Stroke as EStroke, Ui, Vec2};

use super::{AreaDrag, AreaFrame, AreaSnapshot, modules, place};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::file_title;

/// How many props a fresh scatter strews on each square metre.
pub const DENSITY: f32 = 0.25;

pub fn input(editor: &mut Editor, ui: &mut Ui, view_key: &str, frame: &AreaFrame) {
    let response = frame.response;
    let fine = ui.input(|i| EditorKey::Unsnapped.held(i));
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(ground) = frame.drag_ground()
    {
        let view = editor.views.area.view_mut(view_key);
        view.set_drag(AreaDrag::Scatter {
            from: place::snap(ground, fine),
        });
        return;
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        let view = editor.views.area.view_mut(view_key);
        let Some(AreaDrag::Scatter { from }) = view.take_drag() else {
            return;
        };
        let Some(to) = frame.drag_ground().map(|ground| place::snap(ground, fine)) else {
            return;
        };
        let region = Rect::from_points([from, to]);
        if region.size.width < 0.5 || region.size.height < 0.5 {
            editor.toast("Drag a rectangle at least half a metre across to scatter over");
            return;
        }
        let Some(prop) = editor.views.area.view(view_key).prop().map(str::to_owned) else {
            editor.toast("Pick a prop in the Palette to scatter");
            return;
        };
        scatter(editor, frame, &prop, region);
    }
}

/// Strews `prop` over `region` as a new scatter module of the bundle.
pub fn scatter(
    editor: &mut Editor,
    frame: &AreaSnapshot,
    prop: &str,
    region: Rect<engine::core::units::Meters>,
) {
    let Some(id) = frame.reader.id_of::<PropDef>(prop) else {
        editor.toast(format!("{prop} is not a prop that loads"));
        return;
    };
    let module = AreaModule::Scatter(Scatter {
        props: vec![id],
        region,
        per_square_meter: DENSITY,
        seed: Rng::random_seed(),
        scale: ScaleRange::default(),
    });
    let label = format!(
        "Scatter {} over {} m² of {}",
        file_title(prop),
        region.area().round(),
        file_title(&frame.bundle)
    );
    let bundle = frame.bundle.clone();
    if let modules::PlaceOutcome::Partly { reason, .. } = modules::place_modules(
        editor,
        &bundle,
        vec![SceneNode::Module(ModuleNode::new(module))],
        &label,
    ) {
        editor.toast(reason);
    }
}

pub fn overlay(editor: &Editor, painter: &egui::Painter, view_key: &str, frame: &AreaFrame) {
    let view = editor.views.area.view(view_key);
    let color = super::overlays::module_color(Some(engine::systems::area::AreaModuleKind::Scatter));
    let fine = painter.ctx().input(|i| EditorKey::Unsnapped.held(i));
    if let Some(AreaDrag::Scatter { from }) = view.drag()
        && let Some(to) = frame.drag_ground()
    {
        let region = Rect::from_points([*from, place::snap(to, fine)]);
        painter.add(egui::Shape::dashed_line(
            &frame.canvas.outline(region),
            EStroke::new(1.5, color),
            6.0,
            4.0,
        ));
        frame.labels.chip(
            frame.canvas.rect(region).min + Vec2::new(0.0, -4.0),
            format!(
                "{} × {} m · about {} {}",
                region.size.width,
                region.size.height,
                (region.area() * DENSITY).ceil(),
                view.prop().map(file_title).unwrap_or("props")
            ),
            color,
        );
        return;
    }
    let hint = match view.prop() {
        Some(prop) => format!("drag a rectangle to scatter {}", file_title(prop)),
        None => "pick a prop in the Palette, then drag a rectangle to scatter it".to_owned(),
    };
    if let Some(ground) = frame.pointer_ground {
        frame.labels.chip(
            frame.canvas.to_screen(ground) + Vec2::new(10.0, -6.0),
            hint,
            color,
        );
    }
}
