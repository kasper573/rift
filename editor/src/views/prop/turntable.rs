//! The turntable: the prop on its stage through the game's renderer, turning on its own until
//! the pointer takes the camera.

use engine::core::geometry::Degrees;
use engine::systems::area::AreaModuleKind;
use engine::systems::area::prop::PropDef;
use ui::egui::{self, Sense, Stroke, Ui, Vec2};
use ui::theme;

use super::super::area::camera::{self, AreaCanvas};
use super::super::area::overlays::module_color;
use super::super::area::place::draw_footprint;
use super::super::stage;
use crate::app::Editor;

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef, size: Vec2) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let Some(picture) = super::picture(editor, key) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no terrain to stand the prop on",
            egui::FontId::proportional(13.0),
            theme::MUTED,
        );
        return;
    };
    let view_key = format!("prop:{key}");
    stage::show_picture(editor, ui, &painter, rect, &view_key);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let view = editor.views.prop.view_mut(key);
    let busy = camera::navigate(ui, &response, &mut view.turntable, rect.height());
    if busy && response.dragged_by(egui::PointerButton::Secondary) {
        view.spinning = false;
    }
    if response.double_clicked() {
        view.turntable.target = stage::center();
    }
    if view.spinning {
        stage::spin(&mut view.turntable, dt);
        ui.ctx().request_repaint();
    }
    let camera = view.turntable;
    let footprint = view
        .drag
        .as_ref()
        .map_or_else(|| row.footprint.clone(), |drag| drag.footprint.clone());
    let canvas = AreaCanvas::new(rect, camera, picture.1.area.clone());
    draw_footprint(
        &painter,
        &canvas,
        &footprint,
        stage::center(),
        Degrees(0.0),
        1.0,
        Stroke::new(1.5, module_color(Some(AreaModuleKind::Prop))),
    );
    stage::request(editor, ui, &view_key, rect, camera, picture, Vec::new());
}
