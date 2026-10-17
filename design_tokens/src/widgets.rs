use egui::{
    Color32, CornerRadius, FontId, Response, RichText, Sense, Stroke, StrokeKind, Style, Ui, Vec2,
    WidgetInfo, WidgetType,
};

use crate::{CHIP_RADIUS, HAIRLINE, color, family};

/// A heading drawn in capitals that tells the accessibility tree its label as written.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(section_heading(text))
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    ui.add_space(1.0);
}

/// A section's heading as text, for a heading row that carries more than the title.
pub fn section_heading(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(11.0)
        .color(color::MUTED)
        .family(family::medium())
}

/// One row of tabs with the active one underlined. Each tab tells the accessibility tree its own
/// label, as written, though it is drawn in capitals.
pub fn tab_strip<T: PartialEq + Copy>(ui: &mut Ui, tabs: &[(T, String)], active: &mut T) {
    let rule = ui.painter().add(egui::Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (tab, label) in tabs {
            let selected = *tab == *active;
            let caps = label.to_uppercase();
            let font = FontId::new(11.5, family::medium());
            let galley = ui
                .painter()
                .layout_no_wrap(caps.clone(), font.clone(), Color32::WHITE);
            let size = Vec2::new(galley.size().x + 24.0, 30.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label)
            });
            if response.hovered() && !selected {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 8),
                );
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                caps,
                font,
                if selected { color::TEXT } else { color::MUTED },
            );
            if selected {
                ui.painter().line_segment(
                    [
                        rect.left_bottom() + Vec2::new(0.0, -1.0),
                        rect.right_bottom() + Vec2::new(0.0, -1.0),
                    ],
                    Stroke::new(2.0, color::ACCENT),
                );
            }
            if response.has_focus() {
                focus_ring(ui, rect, CornerRadius::ZERO);
            }
            if response.clicked() {
                *active = *tab;
            }
        }
    });
    let y = row.response.rect.bottom() - 1.0;
    let span = ui.max_rect().x_range();
    ui.painter().set(
        rule,
        egui::Shape::line_segment([egui::pos2(span.min, y), egui::pos2(span.max, y)], HAIRLINE),
    );
}

/// A dropdown whose rows answer a hover with their fill alone, as a menu's do.
pub fn combo_box(id_salt: impl egui::AsIdSalt) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id_salt).popup_style(popover.into())
}

pub fn key_hint(ui: &mut Ui, keys: &str) {
    let text = RichText::new(keys)
        .monospace()
        .size(11.0)
        .color(color::MUTED);
    egui::Frame::new()
        .fill(color::INPUT)
        .stroke(HAIRLINE)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(text);
        });
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(30.0, 16.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let checked = *on;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, checked, ""));
    let t = ui.ctx().animate_bool(response.id, *on);
    let fill = if *on { color::ACCENT } else { color::INPUT };
    ui.painter().rect(
        rect,
        8.0,
        fill,
        Stroke::new(1.0, if *on { color::ACCENT } else { color::BORDER }),
        StrokeKind::Inside,
    );
    let knob = egui::pos2(rect.min.x + 8.0 + t * (size.x - 16.0), rect.center().y);
    ui.painter().circle_filled(knob, 5.5, Color32::WHITE);
    if response.has_focus() {
        focus_ring(ui, rect, CornerRadius::same(8));
    }
    response
}

/// egui paints a slider's rail in the inactive widget fill, which the theme gives windows too.
pub fn slider(ui: &mut Ui, slider: egui::Slider) -> Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = color::INPUT;
        ui.add(slider.trailing_fill(true))
    })
    .inner
}

pub fn chip(ui: &mut Ui, text: &str) -> Response {
    chip_colored(ui, text, color::RAISED, color::TEXT)
}

pub fn chip_colored(ui: &mut Ui, text: &str, fill: Color32, ink: Color32) -> Response {
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(HAIRLINE)
        .corner_radius(CHIP_RADIUS)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.5).color(ink));
        })
        .response;
    response.interact(Sense::click())
}

pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            40,
        ))
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120),
        ))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(13.0).color(color).monospace());
        })
        .response
}

pub fn primary(ui: &mut Ui, text: &str) -> Response {
    let response = ui.add(
        egui::Button::new(
            RichText::new(text)
                .color(Color32::WHITE)
                .family(family::medium()),
        )
        .fill(color::ACCENT)
        .stroke(Stroke::new(HAIRLINE.width, color::ACCENT)),
    );
    if response.has_focus() {
        focus_ring(
            ui,
            response.rect,
            ui.visuals().widgets.inactive.corner_radius,
        );
    }
    response
}

pub fn danger(ui: &mut Ui, text: &str) -> Response {
    let response = ui.add(
        egui::Button::new(RichText::new(text).color(color::DANGER))
            .stroke(Stroke::new(1.0, color::DANGER)),
    );
    if response.has_focus() {
        focus_ring(
            ui,
            response.rect,
            ui.visuals().widgets.inactive.corner_radius,
        );
    }
    response
}

pub fn card_frame(selected: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(color::RAISED)
        .stroke(Stroke::new(
            1.0,
            if selected {
                color::ACCENT
            } else {
                color::BORDER
            },
        ))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

fn focus_ring(ui: &Ui, rect: egui::Rect, corner_radius: CornerRadius) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        corner_radius,
        Stroke::new(1.5, color::SELECTION),
        StrokeKind::Outside,
    );
}

fn popover(style: &mut Style) {
    let widgets = &mut style.visuals.widgets;
    for widget in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
    ] {
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
    }
}
