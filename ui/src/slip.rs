use egui::epaint::Shadow;
use egui::{Color32, CornerRadius, InnerResponse, Margin, Painter, Rect, Shape, Ui};

use crate::sheet;

pub const PADDING: Margin = Margin::symmetric(12, 8);

pub const SHADOW: Shadow = Shadow {
    offset: [0, 3],
    blur: 5,
    spread: 0,
    color: Color32::from_black_alpha(115),
};

pub fn show<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    show_tinted(ui, Color32::WHITE, add)
}

fn show_tinted<R>(ui: &mut Ui, tint: Color32, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let under = ui.painter().add(Shape::Noop);
    let shown = egui::Frame::new().inner_margin(PADDING).show(ui, add);
    let rect = shown.response.rect;
    ui.painter().set(under, Shape::Vec(shapes(ui.ctx(), rect)));
    if tint != Color32::WHITE {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, tint);
    }
    shown
}

pub fn paint(painter: &Painter, rect: Rect) {
    painter.add(Shape::Vec(shapes(painter.ctx(), rect)));
}

pub fn frame() -> egui::Frame {
    egui::Frame::new()
        .fill(crate::ink::CREAM)
        .shadow(SHADOW)
        .inner_margin(PADDING)
}

fn shapes(ctx: &egui::Context, rect: Rect) -> Vec<Shape> {
    vec![
        Shape::Rect(SHADOW.as_shape(rect, CornerRadius::ZERO)),
        sheet::vellum_shape(ctx, rect),
    ]
}
