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
/// The colour of what holds the keyboard's focus.
pub const FOCUS: Color32 = ink::GOLD;
pub const SLOT: f32 = 38.0;
pub const SLOT_ICON: f32 = 32.0;
const PILL_HEIGHT: f32 = 20.0;
const NOTICE_PADDING: f32 = 12.0;
const BEAD: f32 = 6.0;

pub fn button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button_look(ui, text.into(), Marked::Not, Look::Outline)
}

/// A button that may be pressed only while `enabled`.
pub fn button_if(ui: &mut Ui, enabled: bool, text: impl Into<WidgetText>) -> Response {
    ui.add_enabled_ui(enabled, |ui| button(ui, text)).inner
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

/// A checkbox reading `name` beside it, either of them switching it.
pub fn toggle(ui: &mut Ui, name: impl Into<WidgetText>, on: &mut bool) -> Response {
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

/// A checkbox with no words beside it.
pub fn tick(ui: &mut Ui, on: &mut bool) -> Response {
    ui.add(egui::Checkbox::without_text(on))
}

/// A checkbox with no words beside it that shows, by a dash, that what it stands for is only
/// partly on.
pub fn tick_mixed(ui: &mut Ui, on: &mut bool, mixed: bool) -> Response {
    ui.add(egui::Checkbox::without_text(on).indeterminate(mixed))
}

/// A checkbox with `name` after it, as a choice in a list or a menu reads.
pub fn check(ui: &mut Ui, on: &mut bool, name: impl Into<WidgetText>) -> Response {
    ui.checkbox(on, name)
}

/// As wide as the longest label a form holds, so the fields of every form on a page line up.
pub const FORM_LABELS: f32 = 196.0;
pub const FORM_SPACING: Vec2 = Vec2::new(16.0, 8.0);

/// A dropdown whose list is whatever its `show_ui` closure draws. Its button is the one every
/// dropdown has: [`CONTROL_HEIGHT`] tall, whatever it shows, so dropdowns side by side line up.
pub fn combo(id_salt: impl egui::AsIdSalt) -> Combo {
    Combo {
        id: egui::Id::new(id_salt),
        text: WidgetText::default(),
        name: None,
        fits: Vec::new(),
        width: None,
        truncate: false,
    }
}

pub struct Combo {
    id: egui::Id,
    text: WidgetText,
    name: Option<String>,
    fits: Vec<WidgetText>,
    width: Option<f32>,
    truncate: bool,
}

impl Combo {
    /// What the button shows.
    pub fn selected_text(mut self, text: impl Into<WidgetText>) -> Combo {
        self.text = text.into();
        self
    }

    /// What a screen reader calls the button, where no label beside it names it.
    pub fn named(mut self, name: impl Into<String>) -> Combo {
        self.name = Some(name.into());
        self
    }

    /// Keeps the button as wide as the widest of `texts` needs, so picking another of them never
    /// changes its width.
    pub fn fitting<T: Into<WidgetText>>(mut self, texts: impl IntoIterator<Item = T>) -> Combo {
        self.fits = texts.into_iter().map(Into::into).collect();
        self
    }

    /// The button's least width; it grows to hold its text.
    pub fn width(mut self, width: f32) -> Combo {
        self.width = Some(width);
        self
    }

    /// Keeps the button `width` wide, cutting its text short where it doesn't fit.
    pub fn truncate(mut self) -> Combo {
        self.truncate = true;
        self
    }

    /// Draws the button, and under it, while it is open, the list `list` draws.
    pub fn show_ui<R>(
        self,
        ui: &mut Ui,
        list: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<Option<R>> {
        let id = ui.make_persistent_id(self.id);
        let popup = id.with("popup");
        let open = egui::Popup::is_id_open(ui.ctx(), popup);
        let padding = ui.spacing().button_padding;
        let caret = ui.spacing().icon_spacing + ui.spacing().icon_width;
        let fitted = self
            .fits
            .iter()
            .map(|text| {
                text.clone()
                    .into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Button,
                    )
                    .size()
                    .x
                    + 2.0 * padding.x
                    + caret
            })
            .fold(0.0, f32::max);
        let least = self.width.unwrap_or(ui.spacing().combo_width).max(fitted);
        let wrap = if self.truncate {
            egui::TextWrapMode::Truncate
        } else {
            egui::TextWrapMode::Extend
        };
        let room = if self.truncate {
            least - 2.0 * padding.x - caret
        } else {
            f32::INFINITY
        };
        let text = self.text.clone();
        let galley = text.into_galley(ui, Some(wrap), room, egui::TextStyle::Button);
        let width = if self.truncate {
            least
        } else {
            (galley.size().x + 2.0 * padding.x + caret).max(least)
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::click());
        response.widget_info(|| {
            let mut info = WidgetInfo::new(WidgetType::ComboBox);
            info.enabled = ui.is_enabled();
            info.label = self.name.clone();
            info.current_text_value = Some(self.text.text().to_owned());
            info
        });
        if ui.is_rect_visible(rect) {
            let visuals = if open {
                ui.visuals().widgets.open
            } else {
                *ui.style().interact(&response)
            };
            let shown = paint_combo_button(ui, rect, &visuals, false);
            let at = Align2::LEFT_CENTER.align_size_within_rect(galley.size(), shown);
            ui.painter()
                .with_clip_rect(inside_stroke(rect, &visuals))
                .galley(at.min, galley, visuals.text_color());
        }
        let list = egui::Popup::menu(&response)
            .id(popup)
            .width(rect.width())
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .style(egui::style::StyleModifier::from(
                crate::popup_style as fn(&mut egui::Style),
            ))
            .show(|ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(ui.spacing().combo_height)
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        list(ui)
                    })
                    .inner
            })
            .map(|shown| shown.inner);
        InnerResponse {
            inner: list,
            response,
        }
    }
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

/// What a drawn combo answers: its button, and the place of the choice picked.
pub struct DrawnCombo {
    pub response: Response,
    pub picked: Option<usize>,
}

/// What a [`search_combo`] says in words: what its field invites before a choice is made or as it
/// is typed in, and what its list says when nothing it holds matches what was typed.
#[derive(Clone, Copy, Debug)]
pub struct SearchWords<'a> {
    pub hint: &'a str,
    pub none_found: &'a str,
}

/// Choices that show as their names alone.
pub struct NamedChoices<'a>(pub &'a [String]);

impl DrawnChoices for NamedChoices<'_> {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn name(&self, place: usize) -> String {
        self.0.get(place).cloned().unwrap_or_default()
    }

    fn widest(&self, ui: &Ui, _part: ComboPart) -> f32 {
        self.0
            .iter()
            .map(|name| {
                ui.painter()
                    .layout_no_wrap(
                        name.clone(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        ink::INK,
                    )
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
    }

    fn draw(&self, ui: &mut Ui, place: usize, _part: ComboPart) {
        if let Some(name) = self.0.get(place) {
            ui.add(
                egui::Label::new(name.as_str())
                    .truncate()
                    .show_tooltip_when_elided(false),
            );
        }
    }
}

/// A combo over `choices`, as wide as the widest of them on its button, whose list is as wide as
/// its widest row so no choice wraps.
pub fn drawn_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    choices: &dyn DrawnChoices,
    selected: usize,
) -> DrawnCombo {
    let width = combo_width(ui, choices);
    shown_combo(ui, id_salt, choices, Some(selected), None, width)
}

/// A combo box over `choices`: a select whose list can be scrolled through and picked from, or
/// narrowed by typing, as it takes what is typed the moment it opens. The arrow keys move along
/// what is listed and Enter picks it. `width` wide, or as wide as its widest choice without one;
/// `selected` is shown on it, or its hint while nothing is.
pub fn search_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    choices: &dyn DrawnChoices,
    selected: Option<usize>,
    words: SearchWords,
    width: Option<f32>,
) -> DrawnCombo {
    let width = width.unwrap_or_else(|| combo_width(ui, choices));
    shown_combo(ui, id_salt, choices, selected, Some(words), width)
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
    ui.horizontal(|ui| key_hint_said(ui, keys, text));
}

/// A sentence after the `keys` that do what it says, on as many lines as it takes.
pub fn key_hint_sentence(ui: &mut Ui, keys: &KeyCap, text: &str) {
    ui.horizontal_wrapped(|ui| key_hint_said(ui, keys, text));
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

/// Marks what holds the keyboard's focus along its own edge, so focus never needs room beside it.
pub fn focus_ring(painter: &Painter, rect: Rect, corner_radius: impl Into<CornerRadius>) {
    painter.rect_stroke(
        rect,
        corner_radius,
        Stroke::new(2.0, FOCUS),
        StrokeKind::Inside,
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

fn combo_width(ui: &Ui, choices: &dyn DrawnChoices) -> f32 {
    let padding = ui.spacing().button_padding;
    let caret = ui.spacing().icon_spacing + ui.spacing().icon_width;
    choices.widest(ui, ComboPart::Button) + 2.0 * padding.x + caret
}

/// Paints a combo's button frame and caret at `rect`, and answers the room left for what it shows.
fn paint_combo_button(
    ui: &Ui,
    rect: Rect,
    visuals: &egui::style::WidgetVisuals,
    typing: bool,
) -> Rect {
    let (fill, stroke) = if typing {
        (crate::theme::INPUT, Stroke::new(1.0, FOCUS))
    } else {
        (visuals.weak_bg_fill, visuals.bg_stroke)
    };
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let inner = combo_content(rect, ui.spacing().button_padding);
    let caret =
        Align2::RIGHT_CENTER.align_size_within_rect(Vec2::splat(ui.spacing().icon_width), inner);
    Mark::CaretDown.paint(ui.painter(), caret, visuals.fg_stroke.color);
    Rect::from_min_max(
        inner.min,
        Pos2::new(caret.min.x - ui.spacing().icon_spacing, inner.max.y),
    )
}

/// The room a combo's button or list row leaves for what it draws: all of the control's fixed
/// height but a hairline, so what is drawn taller than a line of text stays inside the control
/// and centred in it.
fn combo_content(rect: Rect, padding: Vec2) -> Rect {
    rect.shrink2(Vec2::new(padding.x, 1.0))
}

/// What a combo box keeps between frames while its list is open: what was typed, and the row
/// lit for the keys to pick.
#[derive(Clone, Default)]
struct ComboState {
    open: bool,
    typed: String,
    lit: Option<usize>,
}

fn shown_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    choices: &dyn DrawnChoices,
    selected: Option<usize>,
    search: Option<SearchWords>,
    width: f32,
) -> DrawnCombo {
    let padding = ui.spacing().button_padding;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::click());
    let id = ui.make_persistent_id(id_salt);
    let mut state: ComboState = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let keyed_open = response.has_focus()
        && ui.input(|input| input.key_pressed(Key::Enter) || input.key_pressed(Key::ArrowDown));
    if (response.clicked() || keyed_open) && !state.open {
        state = ComboState {
            open: true,
            typed: String::new(),
            lit: selected,
        };
    } else if response.clicked() {
        state.open = false;
    }
    response.widget_info(|| {
        let mut info = WidgetInfo::new(WidgetType::ComboBox);
        info.enabled = ui.is_enabled();
        info.current_text_value = selected.map(|place| choices.name(place));
        info
    });
    let listed = shown_places(choices, &state.typed);
    let mut picked = None;
    let mut keyed = false;
    if state.open {
        let moved = ui.input_mut(|input| {
            let down = input.consume_key(egui::Modifiers::NONE, Key::ArrowDown);
            let up = input.consume_key(egui::Modifiers::NONE, Key::ArrowUp);
            let enter = input.consume_key(egui::Modifiers::NONE, Key::Enter);
            let escape = input.consume_key(egui::Modifiers::NONE, Key::Escape);
            (down, up, enter, escape)
        });
        let (down, up, enter, escape) = moved;
        let at = state
            .lit
            .and_then(|lit| listed.iter().position(|&place| place == lit));
        if down || up {
            keyed = true;
            let next = match (at, down) {
                (None, _) => 0,
                (Some(at), true) => (at + 1).min(listed.len().saturating_sub(1)),
                (Some(at), false) => at.saturating_sub(1),
            };
            state.lit = listed.get(next).copied();
        } else if at.is_none() {
            state.lit = listed.first().copied();
        }
        if enter {
            picked = state.lit.filter(|lit| listed.contains(lit));
        }
        if escape {
            state.open = false;
            response.request_focus();
        }
    }
    let typing = state.open && search.is_some();
    if ui.is_rect_visible(rect) {
        let visuals = if state.open {
            ui.visuals().widgets.open
        } else {
            *ui.style().interact(&response)
        };
        let shown = paint_combo_button(ui, rect, &visuals, typing);
        let room = UiBuilder::new()
            .max_rect(shown)
            .layout(Layout::left_to_right(Align::Center));
        let mut room = ui.new_child(room);
        room.shrink_clip_rect(inside_stroke(rect, &visuals));
        match (typing, search, selected) {
            (true, Some(words), _) => {
                let invite =
                    selected.map_or_else(|| words.hint.to_owned(), |place| choices.name(place));
                let typed = room.add(
                    egui::TextEdit::singleline(&mut state.typed)
                        .id(id.with("typed"))
                        .frame(egui::Frame::NONE)
                        .margin(Margin::ZERO)
                        .hint_text(crate::theme::muted(&invite))
                        .desired_width(f32::INFINITY),
                );
                typed.request_focus();
            }
            (_, _, Some(place)) => choices.draw(&mut room, place, ComboPart::Button),
            (_, Some(words), None) => {
                room.label(crate::theme::muted(words.hint));
            }
            (_, None, None) => {}
        }
    }
    if state.open && picked.is_none() {
        let row_width = choices.widest(ui, ComboPart::List) + 2.0 * padding.x;
        let list_width = row_width.max(width - ui.spacing().menu_margin.sum().x)
            + ui.spacing().scroll.allocated_width();
        let shown = egui::Popup::menu(&response)
            .id(id.with("list"))
            .open(true)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .width(list_width + ui.spacing().menu_margin.sum().x)
            .style(egui::style::StyleModifier::from(
                crate::popup_style as fn(&mut egui::Style),
            ))
            .show(|ui| {
                ui.set_min_width(list_width);
                egui::ScrollArea::vertical()
                    .max_height(ui.spacing().combo_height)
                    .show(ui, |ui| {
                        list_rows(
                            ui, choices, &listed, selected, &mut state, keyed, row_width, search,
                        )
                    })
                    .inner
            });
        if let Some(shown) = shown {
            picked = shown.inner;
            let pressed_outside = ui.input(|input| {
                input.pointer.any_pressed()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|at| !shown.response.rect.contains(at) && !rect.contains(at))
            });
            if pressed_outside {
                state.open = false;
            }
        }
    }
    if picked.is_some() {
        state.open = false;
        response.request_focus();
    }
    if !state.open {
        state.typed.clear();
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    DrawnCombo { response, picked }
}

/// The places of the choices whose names hold what was typed, ignoring case: every one while
/// nothing is typed.
fn shown_places(choices: &dyn DrawnChoices, typed: &str) -> Vec<usize> {
    let typed = typed.trim().to_lowercase();
    (0..choices.count())
        .filter(|&place| typed.is_empty() || choices.name(place).to_lowercase().contains(&typed))
        .collect()
}

/// The rows of a combo box's list: the choices `listed`, each run under its heading, the row lit
/// as the pointer or the keys last left it. Answers the place clicked.
#[allow(clippy::too_many_arguments)]
fn list_rows(
    ui: &mut Ui,
    choices: &dyn DrawnChoices,
    listed: &[usize],
    selected: Option<usize>,
    state: &mut ComboState,
    keyed: bool,
    row_width: f32,
    search: Option<SearchWords>,
) -> Option<usize> {
    let padding = ui.spacing().button_padding;
    let row_width = ui.available_width().max(row_width);
    let mut picked = None;
    if listed.is_empty()
        && let Some(words) = search
    {
        ui.add_sized(
            Vec2::new(row_width, CONTROL_HEIGHT),
            egui::Label::new(crate::theme::muted(words.none_found)),
        );
    }
    let mut heading: Option<String> = None;
    let mut next = 0;
    for &place in listed {
        while next <= place {
            if let Some(starts) = choices.heading(next) {
                heading = Some(starts);
            }
            next += 1;
        }
        if let Some(heading) = heading.take() {
            sheet::list_heading(ui, &heading);
        }
        let (row, chosen) =
            ui.allocate_exact_size(Vec2::new(row_width, CONTROL_HEIGHT), Sense::click());
        chosen.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::SelectableLabel,
                true,
                Some(place) == selected,
                choices.name(place),
            )
        });
        if chosen.hovered() && ui.input(|input| input.pointer.delta() != Vec2::ZERO) {
            state.lit = Some(place);
        }
        let lit = state.lit == Some(place);
        if lit && keyed {
            chosen.scroll_to_me(None);
        }
        ui.painter()
            .add(row_marks(row, lit, Some(place) == selected));
        let room = UiBuilder::new()
            .max_rect(combo_content(row, padding))
            .layout(Layout::left_to_right(Align::Center));
        let mut room = ui.new_child(room);
        room.shrink_clip_rect(row);
        choices.draw(&mut room, place, ComboPart::List);
        if chosen.clicked() {
            picked = Some(place);
        }
    }
    picked
}

/// The part of a combo's button within its border, which what the button shows is kept to so
/// it never draws over the border.
fn inside_stroke(rect: Rect, visuals: &egui::style::WidgetVisuals) -> Rect {
    rect.shrink(visuals.bg_stroke.width.max(1.0))
}

fn key_hint_said(ui: &mut Ui, keys: &KeyCap, text: &str) {
    key_hint(ui, keys);
    ui.label(TextRole::Body.rich(text).color(ink::INK2));
}
