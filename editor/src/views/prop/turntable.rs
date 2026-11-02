//! The turntable: the prop on its stage through the game's renderer, turning on its own until
//! the pointer takes the camera.

use engine::core::units::Degrees;
use engine::systems::area::AreaModuleKind;
use engine::systems::area::prop::PropDef;
use ui::egui::{Stroke, Ui, Vec2};

use super::super::area::overlays::module_color;
use super::super::area::shapes::draw_footprint;
use super::super::stage;
use crate::app::Editor;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef, size: Vec2) {
    let Some(pictured) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.as_ref())
        .and_then(|content| super::pictured(content, key))
    else {
        return;
    };
    super::ensure_view(editor, key);
    let mut state = std::mem::take(&mut editor.views.prop.views.view_mut(key).stage);
    let turntable = stage::turntable(
        editor,
        ui,
        &format!("prop:{key}"),
        size,
        &mut state,
        pictured,
    );
    let view = editor.views.prop.views.view_mut(key);
    view.stage = state;
    let Some(turntable) = turntable else {
        return;
    };
    let footprint = view
        .drag
        .as_ref()
        .map_or_else(|| row.footprint.clone(), |drag| drag.footprint.clone());
    draw_footprint(
        &ui.painter_at(turntable.rect),
        &turntable.canvas,
        &footprint,
        stage::center(),
        Degrees(0.0),
        1.0,
        Stroke::new(1.5, module_color(AreaModuleKind::Prop)),
    );
    turntable.stand(editor, ui, Vec::new());
}
