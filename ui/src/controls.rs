use std::ops::RangeInclusive;
use std::sync::Arc;

use egui::emath::Rot2;
use egui::epaint::TextShape;
use egui::{
    Align, Align2, Color32, CornerRadius, InnerResponse, Key, Margin, Painter, Pos2, Rangef, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, WidgetText,
    WidgetType,
};

use crate::face::{Face, TextRole};
use crate::glyph::{self, Mark};
use crate::{drag, ink, paint, sheet};

/// Every control stands this tall, so an icon-only button lines up with the text buttons beside it.
pub const CONTROL_HEIGHT: f32 = 26.0;
pub(crate) const BUTTON_PADDING: Vec2 = Vec2::new(12.0, 4.0);
pub const RADIUS: f32 = 4.0;
pub const SLOT: f32 = 38.0;
pub const SLOT_ICON: f32 = 32.0;
const PILL_HEIGHT: f32 = 20.0;
const BEAD: f32 = 6.0;

pub fn button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Outline)
}

pub fn primary(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Primary)
}

pub fn danger(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Danger)
}

pub fn faded(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Faded)
}

pub fn marked(ui: &mut Ui, mark: Mark, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Before(mark), Look::Outline)
}

pub fn marked_primary(ui: &mut Ui, mark: Mark, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Before(mark), Look::Primary)
}

pub fn marked_danger(ui: &mut Ui, mark: Mark, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Before(mark), Look::Danger)
}

/// A button that opens a menu or a list to choose from, its caret on the right.
pub fn dropdown(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(
        ui,
        text.into(),
        Marked::After(Mark::CaretDown),
        Look::Outline,
    )
}

pub fn dropdown_primary(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(
        ui,
        text.into(),
        Marked::After(Mark::CaretDown),
        Look::Primary,
    )
}

pub fn slider(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>) -> Response {
    let width = ui.spacing().slider_width;
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::click_and_drag());
    let (low, high) = (*range.start(), *range.end());
    let span = (high - low).max(f32::EPSILON);
    let rail = Rangef::new(rect.min.x + BEAD, rect.max.x - BEAD);
    let step = span / 100.0;
    let mut wanted = None;
    if (response.dragged() || response.is_pointer_button_down_on())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let t = ((pointer.x - rail.min) / rail.span()).clamp(0.0, 1.0);
        wanted = Some(low + t * span);
    }
    if response.has_focus() {
        let (left, right) = ui.input(|input| {
            (
                input.key_pressed(Key::ArrowLeft),
                input.key_pressed(Key::ArrowRight),
            )
        });
        if left {
            wanted = Some(*value - step);
        }
        if right {
            wanted = Some(*value + step);
        }
    }
    if let Some(wanted) = wanted {
        let wanted = rounded_to(wanted.clamp(low.min(high), high.max(low)), step);
        if wanted != *value {
            *value = wanted;
            response.mark_changed();
        }
    }
    let shown = *value;
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(shown), ""));
    let t = ((shown - low) / span).clamp(0.0, 1.0);
    let y = rect.center().y;
    let at = Pos2::new(rail.min + rail.span() * t, y);
    let painter = ui.painter();
    painter.hline(rail, y, Stroke::new(1.0, ink::ink_wash(0.35)));
    painter.line_segment([Pos2::new(rail.min, y), at], Stroke::new(2.0, ink::RUBRIC));
    painter.circle(at, BEAD, ink::GOLD, Stroke::new(1.0, ink::INK));
    if response.has_focus() {
        focus_ring(
            painter,
            Rect::from_center_size(at, Vec2::splat(BEAD * 2.0)),
            BEAD,
        );
    }
    response
}

/// A choice among every variant of `T`, labelled by `label`, answering the one picked when it
/// isn't `current`.
pub fn variant_picker<T: strum::VariantArray + AsRef<str> + Copy + PartialEq>(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    current: T,
    label: egui::Id,
) -> Option<T> {
    let mut picked = current;
    combo(id_salt)
        .selected_text(current.as_ref())
        .show_ui(ui, |ui| {
            for &variant in T::VARIANTS {
                ui.selectable_value(&mut picked, variant, variant.as_ref());
            }
        })
        .response
        .labelled_by(label);
    (picked != current).then_some(picked)
}

/// A form's rows, each a label and its field, laid out under `id_salt`.
pub fn form(ui: &mut Ui, id_salt: impl egui::AsIdSalt, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .min_col_width(crate::form::KEY_WIDTH)
        .spacing(FORM_SPACING)
        .show(ui, add);
}

const FORM_SPACING: Vec2 = Vec2::new(16.0, 8.0);

pub fn combo(id_salt: impl egui::AsIdSalt) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id_salt)
        .popup_style(egui::style::StyleModifier::from(
            crate::popup_style as fn(&mut egui::Style),
        ))
        .icon(|ui, rect, visuals, _open| {
            Mark::CaretDown.paint(ui.painter(), rect, visuals.fg_stroke.color);
        })
}

/// A combo of `width` showing `selected`, whose rows `rows` draws for the filter typed above them,
/// and returns whether one was picked, which closes it.
pub fn filtered_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    selected: impl Into<WidgetText>,
    width: f32,
    hint: &str,
    rows: impl FnOnce(&mut Ui, &str) -> bool,
) {
    let filter_id = ui.make_persistent_id((&id, "filter"));
    combo(id)
        .selected_text(selected)
        .width(width)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let mut filter: String = ui
                .data_mut(|data| data.get_temp(filter_id))
                .unwrap_or_default();
            ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text(hint)
                    .desired_width(width - 20.0),
            )
            .request_focus();
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            if rows(ui, &filter) {
                ui.close();
            }
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    Gold,
    Rubric,
    Azure,
    Verdigris,
    Tyrian,
    Ink,
}

impl Pill {
    pub fn color(self) -> Color32 {
        match self {
            Pill::Gold => ink::GOLD,
            Pill::Rubric => ink::RUBRIC,
            Pill::Azure => ink::AZURE,
            Pill::Verdigris => ink::VERDIGRIS,
            Pill::Tyrian => ink::TYRIAN,
            Pill::Ink => ink::INK,
        }
    }
}

pub fn pill(ui: &mut Ui, text: &str, pill: Pill) -> Response {
    pill_with(ui, pill, |ui| {
        ui.add(egui::Label::new(text).selectable(false));
    })
    .response
}

pub fn pill_with<R>(ui: &mut Ui, pill: Pill, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let color = pill.color();
    egui::Frame::new()
        .fill(ink::white_wash(0.25))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(PILL_HEIGHT / 2.0)
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.style_mut().override_font_id = Some(Face::BookBold.id(12.0));
                ui.visuals_mut().override_text_color = Some(color);
                add(ui)
            })
            .inner
        })
}

pub fn key_hint(ui: &mut Ui, keys: &str) -> Response {
    let galley = key_galley(ui, keys, ink::INK3);
    let (rect, response) = ui.allocate_exact_size(key_cap_size(&galley), Sense::hover());
    paint_key_cap(ui.painter(), rect, galley, ink::INK3);
    response
}

/// `text` after the `keys` that do it.
pub fn key_hint_text(ui: &mut Ui, keys: &str, text: &str) {
    ui.horizontal(|ui| {
        key_hint(ui, keys);
        ui.label(TextRole::Body.rich(text).color(ink::INK2));
    });
}

/// A button reading `label`, the `keys` that press it too on a cap beside it.
pub fn keyed(ui: &mut Ui, label: &str, keys: Option<&str>, enabled: bool) -> Response {
    let ink = if enabled { ink::INK } else { ink::INK3 };
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), Face::BookMedium.id(14.0), ink);
    let cap = keys.map(|keys| key_galley(ui, keys, if enabled { ink::INK2 } else { ink::INK3 }));
    let cap_size = cap.as_ref().map(key_cap_size);
    let width =
        text.size().x + cap_size.map_or(0.0, |size| size.x + KEYED_GAP) + 2.0 * BUTTON_PADDING.x;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    let fill = if enabled && response.hovered() {
        ink::ink_wash(0.04)
    } else {
        ink::LIGHT
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(1.0, ink::ink_wash(0.35)),
        StrokeKind::Inside,
    );
    let left = rect.min.x + BUTTON_PADDING.x;
    let text_width = text.size().x;
    painter.galley(
        Pos2::new(left, rect.center().y - text.size().y / 2.0),
        text,
        ink,
    );
    if let (Some(cap), Some(size)) = (cap, cap_size) {
        let at = Pos2::new(
            left + text_width + KEYED_GAP,
            rect.center().y - size.y / 2.0,
        );
        let color = if enabled { ink::INK2 } else { ink::INK3 };
        paint_key_cap(painter, Rect::from_min_size(at, size), cap, color);
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Buttons moving the row at `index` of `len` earlier or later: the index it is moved to.
pub fn reorder(ui: &mut Ui, index: usize, len: usize) -> Option<usize> {
    let mut to = None;
    if index > 0 && ui.small_button("↑").on_hover_text("earlier").clicked() {
        to = Some(index - 1);
    }
    if index + 1 < len && ui.small_button("↓").on_hover_text("later").clicked() {
        to = Some(index + 1);
    }
    to
}

/// A row of the list `list` carried by its handle to another place in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CarriedRow {
    list: egui::Id,
    index: usize,
}

/// The handle the row at `index` of the list `list` is dragged by to another place in it. A click
/// on it is the response's to use.
pub fn drag_handle(ui: &mut Ui, list: egui::Id, index: usize) -> Response {
    let size = Vec2::new(18.0, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "handle"));
    if response.drag_started() {
        drag::carry(ui.ctx(), CarriedRow { list, index });
    }
    let held = drag::carried::<CarriedRow>(ui.ctx()) == Some(CarriedRow { list, index });
    if ui.is_rect_visible(rect) {
        let color = if held || response.hovered() {
            ink::INK2
        } else {
            ink::INK3
        };
        Mark::Grip.paint(ui.painter(), rect, color);
    }
    response.on_hover_cursor(if held {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::Grab
    })
}

/// Where the row carried by its handle through the list `list` would land among `rows`, each
/// the place it holds in the list and where it is drawn, in the order drawn: a line is drawn
/// there, and once the row is let go, the place it came from and the place it takes.
pub fn dropped_row(ui: &Ui, list: egui::Id, rows: &[(usize, Rect)]) -> Option<(usize, usize)> {
    let carried = drag::carried::<CarriedRow>(ui.ctx()).filter(|carried| carried.list == list)?;
    let pointer = ui.ctx().pointer_interact_pos()?;
    let (last, last_drawn) = *rows.last()?;
    let gap = ui.spacing().item_spacing.y / 2.0;
    let (before, line) = rows
        .iter()
        .find(|(_, drawn)| pointer.y < drawn.center().y)
        .map_or((last + 1, last_drawn.bottom() + gap), |(index, drawn)| {
            (*index, drawn.top() - gap)
        });
    let to = if carried.index < before {
        before - 1
    } else {
        before
    };
    if to != carried.index {
        ui.painter()
            .hline(last_drawn.x_range(), line, Stroke::new(2.0, ink::RUBRIC));
    }
    let let_go = ui.input(|input| input.pointer.any_released());
    (let_go && to != carried.index).then_some((carried.index, to))
}

/// A small cross that removes what it sits beside, saying `what` it does on hover.
pub fn remove(ui: &mut Ui, what: &str) -> Response {
    glyph::button(ui, Mark::Cross, 16.0, ink::INK3, what).on_hover_text(what)
}

pub fn stamp(painter: &Painter, center: Pos2, word: &str, color: Color32) {
    let galley = painter.layout_job(TextRole::Section.job(word, color));
    let angle = -8.0_f32.to_radians();
    let half = (galley.size() + Vec2::new(12.0, 4.0)) / 2.0;
    let rot = Rot2::from_angle(angle);
    let corners = vec![
        center + rot * Vec2::new(-half.x, -half.y),
        center + rot * Vec2::new(half.x, -half.y),
        center + rot * Vec2::new(half.x, half.y),
        center + rot * Vec2::new(-half.x, half.y),
    ];
    painter.add(Shape::closed_line(corners, Stroke::new(1.5, color)));
    painter.add(Shape::Text(
        TextShape::new(center, galley, color).with_angle_and_anchor(angle, Align2::CENTER_CENTER),
    ));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLook {
    pub selected: bool,
    pub faint: bool,
    pub failed: bool,
}

pub fn row<R>(ui: &mut Ui, look: RowLook, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let under = ui.painter().add(Shape::Noop);
    let width = ui.available_width();
    let shown = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        ui.set_width(width);
        if look.failed {
            ui.visuals_mut().override_text_color = Some(ink::RUBRIC);
        } else if look.faint {
            ui.visuals_mut().override_text_color = Some(ink::INK3);
        }
        egui::Frame::new()
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, add)
            .inner
    });
    let rect = shown.response.rect;
    let shapes = row_marks(rect, shown.response.hovered(), look.selected);
    ui.painter().set(under, shapes);
    if shown.response.has_focus() {
        focus_ring(ui.painter(), rect, 0.0);
    }
    shown
}

pub fn row_marks(rect: Rect, hovered: bool, selected: bool) -> Shape {
    let mut shapes = Vec::new();
    if hovered || selected {
        shapes.push(Shape::rect_filled(rect, 0.0, ink::gold_wash(0.2)));
    }
    if selected {
        shapes.push(Shape::rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
            0.0,
            ink::RUBRIC,
        ));
    }
    let y = rect.max.y - 0.5;
    shapes.push(Shape::line_segment(
        [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
        Stroke::new(1.0, ink::ink_wash(0.18)),
    ));
    Shape::Vec(shapes)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotLook {
    pub filled: bool,
    pub hovered: bool,
    pub refused: bool,
}

pub fn slot_frame(painter: &Painter, rect: Rect, look: SlotLook) {
    if look.hovered {
        painter.rect(
            rect,
            3.0,
            ink::gold_wash(0.28),
            Stroke::new(1.0, ink::OCHRE),
            StrokeKind::Inside,
        );
    } else if look.filled {
        sheet::sunk(painter, rect, 3.0);
        painter.line_segment(
            [
                rect.left_top() + Vec2::new(1.0, 1.5),
                rect.right_top() + Vec2::new(-1.0, 1.5),
            ],
            Stroke::new(1.0, ink::ink_wash(0.12)),
        );
    } else {
        paint::dashed_rect(
            painter,
            rect,
            Stroke::new(1.0, ink::ink_wash(0.3)),
            3.0,
            3.0,
        );
    }
    if look.refused {
        glyph::hatch(painter, rect.shrink(1.0), ink::ink_wash(0.35));
    }
}

pub fn slot_count(painter: &Painter, rect: Rect, count: u32) {
    glyph::haloed(
        painter,
        rect.right_bottom() - Vec2::new(2.0, 1.0),
        Align2::RIGHT_BOTTOM,
        &count.to_string(),
        TextRole::Count.font_id(),
        ink::INK,
        glyph::Halo {
            color: ink::CREAM,
            width: 1.0,
        },
    );
}

pub fn slot_lozenge(painter: &Painter, rect: Rect) {
    let centre = rect.right_top() + Vec2::new(-6.0, 6.0);
    painter.add(Shape::convex_polygon(
        vec![
            centre + Vec2::new(0.0, -4.0),
            centre + Vec2::new(3.0, 0.0),
            centre + Vec2::new(0.0, 4.0),
            centre + Vec2::new(-3.0, 0.0),
        ],
        ink::GOLD,
        Stroke::new(1.0, ink::wash(ink::CREAM, 0.8)),
    ));
}

pub fn focus_ring(painter: &Painter, rect: Rect, corner_radius: impl Into<CornerRadius>) {
    painter.rect_stroke(
        rect.expand(2.0),
        corner_radius,
        Stroke::new(2.0, ink::GOLD),
        StrokeKind::Outside,
    );
}

const PRIMED_HUM_HZ: f64 = 1.6;
const PRIMED_TURNS_PER_SEC: f64 = 0.7;
const PRIMED_STEPS: usize = 96;
const PRIMED_TAIL: f32 = 0.32;

/// A slot armed and waiting on its target: a green outline humming in and out, with two glows
/// running round it.
pub fn primed_ring(painter: &Painter, rect: Rect, time: f64) {
    let ring = rect.expand(1.5);
    let hum = 0.5 + 0.5 * (time * PRIMED_HUM_HZ * std::f64::consts::TAU).sin() as f32;
    painter.rect_stroke(
        ring,
        3.0,
        Stroke::new(1.0, ink::wash(ink::SPRING, 0.3 + 0.3 * hum)),
        StrokeKind::Middle,
    );
    let turn = (time * PRIMED_TURNS_PER_SEC).fract() as f32;
    let points = perimeter(ring, PRIMED_STEPS);
    for (step, pair) in points.windows(2).enumerate() {
        let along = step as f32 / PRIMED_STEPS as f32;
        let glow: f32 = [turn, (turn + 0.5).fract()]
            .iter()
            .map(|head| {
                let behind = (head - along).rem_euclid(1.0);
                (1.0 - behind / PRIMED_TAIL).max(0.0).powi(2)
            })
            .sum::<f32>()
            .min(1.0);
        if glow <= 0.0 {
            continue;
        }
        let segment = [pair[0], pair[1]];
        let lift = 0.75 + 0.25 * hum;
        painter.line_segment(
            segment,
            Stroke::new(4.0, ink::wash(ink::SPRING, 0.3 * glow * lift)),
        );
        painter.line_segment(
            segment,
            Stroke::new(2.0, ink::wash(ink::SPRING, glow * lift)),
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marked {
    Not,
    Before(Mark),
    After(Mark),
}

enum Look {
    Outline,
    Primary,
    Danger,
    Faded,
}

enum Interaction {
    Idle,
    Hovered,
    Pressed,
}

fn button_look(ui: &mut Ui, text: WidgetText, marked: Marked, look: Look) -> Response {
    let mut job = Arc::unwrap_or_clone(text.into_layout_job(
        ui.style(),
        TextRole::Button.font_id().into(),
        Align::Center,
    ));
    // The look decides the text's ink: a primary's cream would vanish under any ink the text
    // brings.
    for section in &mut job.sections {
        section.format.color = Color32::PLACEHOLDER;
    }
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.painter().layout_job(job);
    let mark_room = match marked {
        Marked::Not => 0.0,
        Marked::Before(_) | Marked::After(_) => 20.0,
    };
    let size = Vec2::new(
        galley.size().x + mark_room + BUTTON_PADDING.x * 2.0,
        CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = galley.text().to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let enabled = ui.is_enabled();
    let hovered = enabled && response.hovered();
    let pressed = hovered && response.is_pointer_button_down_on();
    let state = match (hovered, pressed) {
        (_, true) => Interaction::Pressed,
        (true, false) => Interaction::Hovered,
        (false, false) => Interaction::Idle,
    };
    let painter = ui.painter();
    let (fill, line, text_color) = match (look, enabled) {
        (_, false) | (Look::Faded, true) => (Color32::TRANSPARENT, ink::ink_wash(0.3), ink::INK3),
        (Look::Outline, true) => (
            match state {
                Interaction::Idle => Color32::TRANSPARENT,
                Interaction::Hovered => ink::ink_wash(0.05),
                Interaction::Pressed => ink::ink_wash(0.1),
            },
            ink::INK,
            ink::INK,
        ),
        (Look::Primary, true) => (
            match state {
                Interaction::Idle => ink::INK,
                Interaction::Hovered => ink::INK.lerp_to_gamma(ink::INK2, 0.5),
                Interaction::Pressed => ink::INK2,
            },
            ink::INK,
            ink::CREAM,
        ),
        (Look::Danger, true) => (
            match state {
                Interaction::Idle => Color32::TRANSPARENT,
                Interaction::Hovered => ink::wash(ink::RUBRIC, 0.05),
                Interaction::Pressed => ink::wash(ink::RUBRIC, 0.1),
            },
            ink::RUBRIC,
            ink::RUBRIC,
        ),
    };
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let mut text_at = rect.center() - galley.size() / 2.0;
    let mark_at = |x: f32| Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(13.0));
    match marked {
        Marked::Not => {}
        Marked::Before(mark) => {
            text_at.x += mark_room / 2.0;
            mark.paint(
                painter,
                mark_at(text_at.x - mark_room / 2.0 - 4.0),
                text_color,
            );
        }
        Marked::After(mark) => {
            text_at.x -= mark_room / 2.0;
            let x = text_at.x + galley.size().x + mark_room / 2.0 + 4.0;
            mark.paint(painter, mark_at(x), text_color);
        }
    }
    painter.galley(text_at, galley, text_color);
    if response.has_focus() {
        focus_ring(painter, rect, RADIUS);
    }
    response
}

/// Dashes as plain pixel quads in one mesh: an empty bag is dozens of these, and egui would feather
/// each dash of a dashed line as a stroke of its own.
/// `steps + 1` points evenly spaced round the rectangle's edge, clockwise from its top left corner,
/// the last one back on the first.
fn perimeter(rect: Rect, steps: usize) -> Vec<Pos2> {
    let (width, height) = (rect.width(), rect.height());
    let length = 2.0 * (width + height);
    (0..=steps)
        .map(|step| {
            let reach = (step as f32 / steps as f32 * length) % length;
            if reach < width {
                Pos2::new(rect.min.x + reach, rect.min.y)
            } else if reach < width + height {
                Pos2::new(rect.max.x, rect.min.y + reach - width)
            } else if reach < 2.0 * width + height {
                Pos2::new(rect.max.x - (reach - width - height), rect.max.y)
            } else {
                Pos2::new(rect.min.x, rect.max.y - (reach - 2.0 * width - height))
            }
        })
        .collect()
}

const KEYED_GAP: f32 = 8.0;

fn key_galley(ui: &Ui, keys: &str, color: Color32) -> Arc<egui::Galley> {
    ui.painter().layout_job(TextRole::KeyHint.job(keys, color))
}

/// `value` to the decimals `step` needs, so a slid value reads as plainly as one typed.
fn rounded_to(value: f32, step: f32) -> f32 {
    let decimals = (-step.log10()).ceil().max(0.0);
    let scale = 10f32.powf(decimals);
    (value * scale).round() / scale
}

fn key_cap_size(galley: &Arc<egui::Galley>) -> Vec2 {
    galley.size() + Vec2::new(5.0, 1.0) * 2.0 + Vec2::splat(2.0)
}

fn paint_key_cap(painter: &Painter, rect: Rect, galley: Arc<egui::Galley>, color: Color32) {
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, ink::ink_wash(0.35)),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}
