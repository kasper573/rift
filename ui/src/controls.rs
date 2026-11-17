use std::ops::RangeInclusive;
use std::sync::Arc;

use egui::emath::Rot2;
use egui::epaint::TextShape;
use egui::{
    Align, Align2, Color32, CornerRadius, InnerResponse, Key, Layout, Margin, Painter, Pos2,
    Rangef, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureId, Ui, UiBuilder, Vec2,
    WidgetInfo, WidgetText, WidgetType,
};

use crate::face::{Face, TextRole};
use crate::glyph::{self, Mark};
use crate::keys::{CapFace, KeyCap};
use crate::{drag, ink, paint, sheet};

/// Every control stands this tall, so an icon-only button lines up with the text buttons beside it.
pub const CONTROL_HEIGHT: f32 = 26.0;
/// How far a row's content stands from its edges.
pub const ROW_MARGIN: Margin = Margin::symmetric(8, 5);
pub(crate) const BUTTON_PADDING: Vec2 = Vec2::new(12.0, 4.0);
pub const RADIUS: f32 = 4.0;
pub const SLOT: f32 = 38.0;
pub const SLOT_ICON: f32 = 32.0;
const PILL_HEIGHT: f32 = 20.0;
const NOTICE_PADDING: f32 = 12.0;
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

/// Keeps the words and the size the button had when pressed, so nothing beside it moves while the
/// answer is awaited. Whoever draws it sends nothing on a click.
pub fn waiting(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Waiting)
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
    let step = (range.end() - range.start()).max(f32::EPSILON) / 100.0;
    stepped_slider(ui, value, range, step)
}

/// A slider over whole numbers, which the arrow keys move by one.
pub fn count_slider(ui: &mut Ui, count: &mut u32, range: RangeInclusive<u32>) -> Response {
    let mut value = *count as f32;
    let range = *range.start() as f32..=*range.end() as f32;
    let response = stepped_slider(ui, &mut value, range, 1.0);
    *count = value.round() as u32;
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
        .min_col_width(FORM_LABELS)
        .spacing(FORM_SPACING)
        .show(ui, add);
}

pub fn toggle(ui: &mut Ui, name: &str, on: &mut bool) -> Response {
    let label = ui.add(egui::Label::new(name).sense(Sense::click()));
    let tick = ui
        .add(egui::Checkbox::without_text(on))
        .labelled_by(label.id);
    let mut row = tick.union(label.clone());
    if label.clicked() {
        *on = !*on;
        row.mark_changed();
    }
    row
}

/// As wide as the longest label a form holds, so the fields of every form on a page line up.
pub const FORM_LABELS: f32 = 196.0;
pub const FORM_SPACING: Vec2 = Vec2::new(16.0, 8.0);

pub fn combo(id_salt: impl egui::AsIdSalt) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id_salt)
        .popup_style(egui::style::StyleModifier::from(
            crate::popup_style as fn(&mut egui::Style),
        ))
        .icon(|ui, rect, visuals, _open| {
            Mark::CaretDown.paint(ui.painter(), rect, visuals.fg_stroke.color);
        })
}

/// The choices of a [`drawn_combo`]: each shows as more than words, and runs of them may stand
/// under a heading in the list it opens.
pub trait DrawnChoices {
    fn count(&self) -> usize;
    /// The choice at `place` in words, for a screen reader.
    fn name(&self, place: usize) -> String;
    /// The heading over the run of choices that starts at `place`, where one starts there.
    fn heading(&self, _place: usize) -> Option<String> {
        None
    }
    /// How wide the widest choice is drawn in `part`.
    fn widest(&self, ui: &Ui, part: ComboPart) -> f32;
    fn draw(&self, ui: &mut Ui, place: usize, part: ComboPart);
}

/// Where a [`DrawnChoices`] draws a choice: on the combo's button, or as a row of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboPart {
    Button,
    List,
}

/// What a drawn combo answers: its button, the place of the choice picked, and the row of its
/// list the pointer is on, with that row's place.
pub struct DrawnCombo {
    pub response: Response,
    pub picked: Option<usize>,
    pub pointed: Option<(usize, Response)>,
}

/// A combo over `choices`, as wide as the widest of them on its button, whose list is as wide as
/// its widest row so no choice wraps.
pub fn drawn_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    choices: &dyn DrawnChoices,
    selected: usize,
) -> DrawnCombo {
    let padding = ui.spacing().button_padding;
    let caret = ui.spacing().icon_spacing + ui.spacing().icon_width;
    let width = choices.widest(ui, ComboPart::Button) + 2.0 * padding.x + caret;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::click());
    let id = ui.make_persistent_id(id_salt);
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    response.widget_info(|| {
        let mut info = WidgetInfo::new(WidgetType::ComboBox);
        info.enabled = ui.is_enabled();
        info.current_text_value = Some(choices.name(selected));
        info
    });
    if ui.is_rect_visible(rect) {
        let visuals = if open {
            ui.visuals().widgets.open
        } else {
            *ui.style().interact(&response)
        };
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(padding);
        let caret = egui::Align2::RIGHT_CENTER
            .align_size_within_rect(Vec2::splat(ui.spacing().icon_width), inner);
        Mark::CaretDown.paint(ui.painter(), caret, visuals.fg_stroke.color);
        let shown = Rect::from_min_max(
            inner.min,
            Pos2::new(caret.min.x - ui.spacing().icon_spacing, inner.max.y),
        );
        let room = UiBuilder::new()
            .max_rect(shown)
            .layout(Layout::left_to_right(Align::Center));
        choices.draw(&mut ui.new_child(room), selected, ComboPart::Button);
    }
    let row_width = choices.widest(ui, ComboPart::List) + 2.0 * padding.x;
    let list_width = row_width.max(width - ui.spacing().menu_margin.sum().x)
        + ui.spacing().scroll.allocated_width();
    let listed = egui::Popup::menu(&response)
        .id(id)
        .width(list_width + ui.spacing().menu_margin.sum().x)
        .style(egui::style::StyleModifier::from(
            crate::popup_style as fn(&mut egui::Style),
        ))
        .show(|ui| {
            ui.set_min_width(list_width);
            egui::ScrollArea::vertical()
                .max_height(ui.spacing().combo_height)
                .show(ui, |ui| {
                    let row_width = ui.available_width().max(row_width);
                    let mut picked = None;
                    let mut pointed = None;
                    for place in 0..choices.count() {
                        if let Some(heading) = choices.heading(place) {
                            sheet::list_heading(ui, &heading);
                        }
                        let (row, chosen) = ui.allocate_exact_size(
                            Vec2::new(row_width, CONTROL_HEIGHT),
                            Sense::click(),
                        );
                        chosen.widget_info(|| {
                            WidgetInfo::selected(
                                WidgetType::SelectableLabel,
                                true,
                                place == selected,
                                choices.name(place),
                            )
                        });
                        ui.painter()
                            .add(row_marks(row, chosen.hovered(), place == selected));
                        let room = UiBuilder::new()
                            .max_rect(row.shrink2(padding))
                            .layout(Layout::left_to_right(Align::Center));
                        choices.draw(&mut ui.new_child(room), place, ComboPart::List);
                        if chosen.clicked() {
                            picked = Some(place);
                        }
                        if chosen.hovered() {
                            pointed = Some((place, chosen));
                        }
                    }
                    (picked, pointed)
                })
                .inner
        })
        .map(|shown| shown.inner);
    let (picked, pointed) = listed.unwrap_or((None, None));
    DrawnCombo {
        response,
        picked,
        pointed,
    }
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
            Pill::Gold => ink::OCHRE,
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

pub fn pill_with<R>(
    ui: &mut Ui,
    pill: Pill,
    mut add: impl FnMut(&mut Ui) -> R,
) -> InnerResponse<R> {
    let color = pill.color();
    let mut framed = |ui: &mut Ui| {
        egui::Frame::new()
            .fill(ink::white_wash(0.25))
            .stroke(Stroke::new(1.0, color))
            .corner_radius(PILL_HEIGHT / 2.0)
            .inner_margin(Margin::symmetric(8, 1))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.style_mut().override_font_id = Some(TextRole::Pill.font_id());
                    ui.visuals_mut().override_text_color = Some(color);
                    add(ui)
                })
                .inner
            })
    };
    if !ui.layout().main_wrap {
        return framed(ui);
    }
    // A frame is laid out where its row stands, whatever room is left there, so a row that wraps
    // is told the pill's size first and takes it to the next row when it doesn't fit.
    let size = {
        let measuring = UiBuilder::new().id_salt("pill size").invisible();
        framed(&mut ui.new_child(measuring)).response.rect.size()
    };
    ui.allocate_ui(size, framed).inner
}

pub fn notice(ui: &mut Ui, text: &str) -> Response {
    let font = TextRole::Section.font_id();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ink::OCHRE);
    let size = Vec2::new(galley.size().x + 2.0 * NOTICE_PADDING, PILL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    let radius = CornerRadius::same((PILL_HEIGHT / 2.0) as u8);
    let fill = if response.hovered() {
        ink::LIGHT
    } else {
        ink::CREAM
    };
    let painter = ui.painter();
    painter.add(crate::slip::SHADOW.as_shape(rect, radius));
    painter.rect(
        rect,
        radius,
        fill,
        Stroke::new(1.5, ink::OCHRE),
        StrokeKind::Inside,
    );
    painter.galley(
        Align2::CENTER_CENTER
            .anchor_size(rect.center(), galley.size())
            .min,
        galley,
        ink::OCHRE,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn key_hint(ui: &mut Ui, cap: &KeyCap) -> Response {
    let laid = LaidCap::new(ui, cap, ink::INK3);
    let (rect, response) = ui.allocate_exact_size(laid.size(), Sense::hover());
    laid.paint(ui.painter(), rect, ink::INK3);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &cap.name));
    response.on_hover_text(&cap.name)
}

/// `text` after the `keys` that do it.
pub fn key_hint_text(ui: &mut Ui, keys: &KeyCap, text: &str) {
    ui.horizontal(|ui| {
        key_hint(ui, keys);
        ui.label(TextRole::Body.rich(text).color(ink::INK2));
    });
}

#[derive(Clone, Copy)]
pub struct KeyHint<'a> {
    pub keys: &'a KeyCap,
    pub does: &'a str,
}

pub fn hint_line(ui: &mut Ui, hints: &[KeyHint], leaves: Option<KeyHint>) {
    let row = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(ui.available_width(), HINT_LINE_HEIGHT),
    );
    let along = Layout::left_to_right(Align::Center);
    let mut beside = row;
    if let Some(leaves) = leaves {
        let at = Rect::from_min_max(
            Pos2::new(row.max.x - hint_width(ui, &leaves), row.min.y),
            row.max,
        );
        ui.scope_builder(UiBuilder::new().max_rect(at).layout(along), |ui| {
            key_hint_small(ui, leaves);
        });
        beside.max.x = at.min.x - HINT_GAP;
    }
    ui.scope_builder(UiBuilder::new().max_rect(beside).layout(along), |ui| {
        ui.spacing_mut().item_spacing.x = HINT_GAP;
        for hint in hints {
            if hint_width(ui, hint) > ui.available_width() {
                break;
            }
            key_hint_small(ui, *hint);
        }
    });
    ui.allocate_rect(row, Sense::hover());
}

pub const HINT_LINE_HEIGHT: f32 = 22.0;
const HINT_GAP: f32 = 14.0;
const HINT_INNER_GAP: f32 = 6.0;

fn key_hint_small(ui: &mut Ui, hint: KeyHint) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = HINT_INNER_GAP;
        key_hint(ui, hint.keys);
        ui.label(TextRole::Small.rich(hint.does).color(ink::INK2));
    });
}

fn hint_width(ui: &Ui, hint: &KeyHint) -> f32 {
    let cap = LaidCap::new(ui, hint.keys, ink::INK3).size().x;
    let does = ui
        .painter()
        .layout_no_wrap(hint.does.to_owned(), TextRole::Small.font_id(), ink::INK2)
        .size()
        .x;
    cap + HINT_INNER_GAP + does
}

/// A button reading `label`, the `keys` that press it too on a cap beside it.
pub fn keyed(ui: &mut Ui, label: &str, keys: Option<&KeyCap>, enabled: bool) -> Response {
    let ink = if enabled { ink::INK } else { ink::INK3 };
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), Face::BookMedium.id(14.0), ink);
    let cap = keys.map(|keys| LaidCap::new(ui, keys, if enabled { ink::INK2 } else { ink::INK3 }));
    let cap_size = cap.as_ref().map(LaidCap::size);
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
        cap.paint(painter, Rect::from_min_size(at, size), color);
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
    let written_from = center - galley.rect.center().to_vec2();
    painter.add(Shape::Text(
        TextShape::new(written_from, galley, color)
            .with_angle_and_anchor(angle, Align2::CENTER_CENTER),
    ));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLook {
    pub selected: bool,
    pub faint: bool,
    pub failed: bool,
    pub hover: RowHover,
}

/// Whether the pointer over a row lights it. A list whose selection the pointer moves lights its
/// selection alone, so a pointer at rest on one row never lights a second beside the one the keys
/// moved on to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowHover {
    #[default]
    Lit,
    Unlit,
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
            .inner_margin(ROW_MARGIN)
            .show(ui, add)
            .inner
    });
    let rect = shown.response.rect;
    let hovered = look.hover == RowHover::Lit && shown.response.hovered();
    let shapes = row_marks(rect, hovered, look.selected);
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
    /// The two colours a filled slot is edged in, from its top to its bottom.
    pub edge: Option<[Color32; 2]>,
}

pub const SLOT_EDGE: f32 = 2.5;
pub const SLOT_PIP: f32 = 10.0;

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
    if look.filled
        && let Some(edge) = look.edge
    {
        paint::gradient_edge(painter, rect, SLOT_EDGE, edge);
    }
    if look.refused {
        glyph::hatch(painter, rect.shrink(1.0), ink::ink_wash(0.35));
    }
}

/// A small picture in the top left corner of a slot, clear of its lozenge and its count.
pub fn slot_pip(painter: &Painter, rect: Rect, texture: TextureId, tint: Color32) {
    painter.image(
        texture,
        Rect::from_min_size(rect.min + Vec2::splat(SLOT_EDGE), Vec2::splat(SLOT_PIP)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
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
    Waiting,
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
    let enabled = ui.is_enabled() && !matches!(look, Look::Waiting);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = enabled && response.hovered();
    let pressed = hovered && response.is_pointer_button_down_on();
    let state = match (hovered, pressed) {
        (_, true) => Interaction::Pressed,
        (true, false) => Interaction::Hovered,
        (false, false) => Interaction::Idle,
    };
    let painter = ui.painter();
    let dashed = matches!(look, Look::Waiting);
    let (fill, line, text_color) = match (look, enabled) {
        (Look::Waiting, _) => (Color32::TRANSPARENT, ink::ink_wash(0.5), ink::INK2),
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
    if dashed {
        paint::dashed_rect(painter, rect, Stroke::new(1.0, line), 3.0, 3.0);
    } else {
        painter.rect(
            rect,
            RADIUS,
            fill,
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
    }
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

fn stepped_slider(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, step: f32) -> Response {
    let width = ui.spacing().slider_width;
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::click_and_drag());
    let (low, high) = (*range.start(), *range.end());
    let span = (high - low).max(f32::EPSILON);
    let rail = Rangef::new(rect.min.x + BEAD, rect.max.x - BEAD);
    let mut wanted = None;
    if (response.dragged() || response.is_pointer_button_down_on() || response.clicked())
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

/// `value` to the decimals `step` needs, so a slid value reads as plainly as one typed.
fn rounded_to(value: f32, step: f32) -> f32 {
    let decimals = (-step.log10()).ceil().max(0.0);
    let scale = 10f32.powf(decimals);
    (value * scale).round() / scale
}

/// A key cap laid out to paint: a cap for each modifier held, joined by "+", then its face.
pub(crate) struct LaidCap {
    parts: Vec<CapPart>,
}

enum CapPart {
    Cap(Arc<egui::Galley>),
    Join(Arc<egui::Galley>),
}

impl LaidCap {
    pub(crate) fn new(ui: &Ui, cap: &KeyCap, color: Color32) -> LaidCap {
        let painter = ui.painter();
        let mut parts = Vec::new();
        for face in cap.faces() {
            if !parts.is_empty() {
                let join = painter.layout_job(TextRole::KeyHint.job("+", color));
                parts.push(CapPart::Join(join));
            }
            parts.push(CapPart::Cap(painter.layout_job(face_job(&face, color))));
        }
        LaidCap { parts }
    }

    pub(crate) fn size(&self) -> Vec2 {
        let sizes = self.parts.iter().map(CapPart::size);
        let gaps = CAP_JOIN_GAP * self.parts.len().saturating_sub(1) as f32;
        sizes.fold(Vec2::new(gaps, 0.0), |sum, size| {
            Vec2::new(sum.x + size.x, sum.y.max(size.y))
        })
    }

    pub(crate) fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let mut left = rect.min.x;
        for part in self.parts {
            let size = part.size();
            let at = Rect::from_min_size(Pos2::new(left, rect.center().y - size.y / 2.0), size);
            match part {
                CapPart::Cap(galley) => {
                    painter.rect_stroke(
                        at,
                        3.0,
                        Stroke::new(1.0, ink::ink_wash(0.5)),
                        StrokeKind::Inside,
                    );
                    painter.galley(at.center() - galley.size() / 2.0, galley, color);
                }
                CapPart::Join(galley) => {
                    painter.galley(at.min, galley, color);
                }
            }
            left += size.x + CAP_JOIN_GAP;
        }
    }
}

impl CapPart {
    fn size(&self) -> Vec2 {
        match self {
            CapPart::Cap(galley) => galley.size() + Vec2::new(5.0, 1.0) * 2.0 + Vec2::splat(2.0),
            CapPart::Join(galley) => galley.size(),
        }
    }
}

const CAP_JOIN_GAP: f32 = 3.0;

fn face_job(face: &CapFace, color: Color32) -> egui::text::LayoutJob {
    let letters = TextRole::KeyHint.format(color);
    let mut job = egui::text::LayoutJob::default();
    for run in face.runs() {
        let glyph = run.scale != 1.0;
        let format = egui::TextFormat {
            font_id: egui::FontId::new(letters.font_id.size * run.scale, run.family),
            valign: Align::Center,
            line_height: if glyph { None } else { letters.line_height },
            extra_letter_spacing: if glyph {
                0.0
            } else {
                letters.extra_letter_spacing
            },
            ..letters.clone()
        };
        job.append(&run.text, 0.0, format);
    }
    job
}
