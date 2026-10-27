//! Chips: small framed labels that pick, filter or say a mode, set in a row of controls.

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2,
};

use crate::controls::{self, Pill};
use crate::glyph::{self, Mark};
use crate::{ink, theme};

pub fn plain(ui: &mut Ui, text: impl Into<RichText>) -> Response {
    colored(ui, text, theme::RAISED, theme::TEXT)
}

pub fn colored(ui: &mut Ui, text: impl Into<RichText>, fill: Color32, ink: Color32) -> Response {
    framed(ui, text.into(), fill, ink, false)
}

/// A chip that opens a list to choose from, its caret on the right.
pub fn dropdown(ui: &mut Ui, text: impl Into<RichText>) -> Response {
    dropdown_colored(ui, text, theme::RAISED, theme::TEXT)
}

pub fn dropdown_colored(
    ui: &mut Ui,
    text: impl Into<RichText>,
    fill: Color32,
    ink: Color32,
) -> Response {
    framed(ui, text.into(), fill, ink, true)
}

/// A chip that is on or off, such as a filter.
pub fn toggle(ui: &mut Ui, text: &str, on: bool) -> Response {
    let (fill, ink) = if on {
        (theme::ACCENT_SOFT, theme::TEXT)
    } else {
        (theme::RAISED, theme::MUTED)
    };
    colored(ui, text, fill, ink)
}

/// One of a few chips of which one is the mode in use.
/// A colour shown as a small square before its name.
pub fn swatch(ui: &mut Ui, text: impl Into<RichText>, colour: Color32) -> Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        ui.painter().rect(
            rect,
            RADIUS,
            colour,
            Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        ui.label(text.into())
    })
    .inner
}

pub fn mode(ui: &mut Ui, text: impl Into<RichText>, on: bool) -> Response {
    let (fill, ink) = if on {
        (ink::LIGHT, ink::RUBRIC)
    } else {
        (ink::ink_wash(0.05), ink::INK2)
    };
    colored(ui, text, fill, ink)
}

/// A pill of what narrows something, and whether its cross was clicked to take it away.
pub fn closable(ui: &mut Ui, text: &str) -> (Response, bool) {
    let mut closed = false;
    let response = controls::pill_with(ui, Pill::Rubric, |ui| {
        ui.label(text);
        if glyph::button(ui, Mark::Cross, 11.0, theme::MUTED, "Remove").clicked() {
            closed = true;
        }
    })
    .response;
    (response.interact(Sense::click()), closed)
}

/// A chip around controls of its own, such as a key to press and a cross that takes it away.
pub fn holding<R>(
    ui: &mut Ui,
    fill: Color32,
    border: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| ui.horizontal(add).inner)
}

/// A label painted where no layout reaches, such as over a canvas, `anchor`ed at `at`.
pub fn floating(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    text: &str,
    fill: Color32,
    ink: Color32,
    size: f32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), ink);
    let rect = anchor.anchor_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(rect, RADIUS, fill);
    painter.rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Outside,
    );
    painter.galley(rect.min + Vec2::new(5.0, 2.0), galley, ink);
    rect
}

const HEIGHT: f32 = 20.0;
const RADIUS: f32 = 3.0;
const PADDING: f32 = 8.0;
const GAP: f32 = 4.0;

/// Drawn at a fixed height under the controls' own, so a chip appearing in a row of controls
/// never makes the row taller.
fn framed(ui: &mut Ui, text: RichText, fill: Color32, ink: Color32, caret: bool) -> Response {
    let galley = |text: RichText| {
        egui::WidgetText::from(text.color(ink)).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
    };
    let label = galley(text.size(12.5));
    let mark = caret.then(|| galley(Mark::CaretDown.text(10.0)));
    let marked = mark.as_ref().map_or(0.0, |mark| mark.size().x + GAP);
    let size = Vec2::new(label.size().x + marked + 2.0 * PADDING, HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.text())
    });
    let painter = ui.painter();
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let left = rect.left() + PADDING;
    let at = |x: f32, height: f32| egui::pos2(x, rect.center().y - height / 2.0);
    let mark_x = left + label.size().x + GAP;
    painter.galley(at(left, label.size().y), label, ink);
    if let Some(mark) = mark {
        painter.galley(at(mark_x, mark.size().y), mark, ink);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
