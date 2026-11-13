use std::fmt::Display;
use std::ops::RangeInclusive;
use std::str::FromStr;

use egui::{Id, KeyboardShortcut, Response, TextEdit, Ui};

use crate::draft::{Typed, typed};
use crate::{controls, keys, theme};

/// The text field: a line, or `rows` of them, typed in the input box.
pub struct InputField<'t> {
    text: &'t mut String,
    id: Option<Id>,
    unit: Option<&'t str>,
    hint: Option<&'t str>,
    rows: Option<usize>,
    width: Option<f32>,
    char_limit: Option<usize>,
    return_key: Option<Option<KeyboardShortcut>>,
    holds_focus: bool,
}

impl<'t> InputField<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            text,
            id: None,
            unit: None,
            hint: None,
            rows: None,
            width: None,
            char_limit: None,
            return_key: None,
            holds_focus: false,
        }
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn unit(mut self, unit: &'t str) -> Self {
        self.unit = Some(unit);
        self
    }

    pub fn hint(mut self, hint: &'t str) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = Some(rows);
        self
    }

    /// Narrower than the room it's in, which it otherwise fills.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn char_limit(mut self, char_limit: usize) -> Self {
        self.char_limit = Some(char_limit);
        self
    }

    /// The key that sends what's typed and leaves the field; none keeps the field on every key.
    pub fn return_key(mut self, return_key: Option<KeyboardShortcut>) -> Self {
        self.return_key = Some(return_key);
        self
    }

    /// Keeps the field focused through the key that would move on to the next widget, for
    /// whoever shows the field to read that key as its own.
    pub fn holds_focus(mut self) -> Self {
        self.holds_focus = true;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let Self {
            text,
            id,
            unit,
            hint,
            rows,
            width,
            char_limit,
            return_key,
            holds_focus,
        } = self;
        within(ui, width, |ui| {
            boxed(ui, unit, |ui| {
                let edit = match rows {
                    Some(rows) => TextEdit::multiline(text).desired_rows(rows),
                    None => TextEdit::singleline(text),
                };
                let mut edit = unframed(edit);
                if let Some(id) = id {
                    edit = edit.id(id);
                }
                if let Some(hint) = hint {
                    edit = edit.hint_text(theme::muted(hint));
                }
                if let Some(char_limit) = char_limit {
                    edit = edit.char_limit(char_limit);
                }
                if let Some(return_key) = return_key {
                    edit = edit.return_key(return_key);
                }
                ui.add(edit.lock_focus(holds_focus))
            })
        })
    }
}

/// Puts the caret of the field `id` after the last character of `text`, the text it holds.
pub fn caret_to_end(ctx: &egui::Context, id: Id, text: &str) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    let end = egui::text::CCursor::new(text.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(end)));
    state.store(ctx, id);
}

/// A number within `range` on a slider, beside the field it can be typed into exactly.
pub struct SliderField<'v> {
    value: &'v mut f32,
    range: RangeInclusive<f32>,
}

impl<'v> SliderField<'v> {
    pub fn new(value: &'v mut f32, range: RangeInclusive<f32>) -> Self {
        Self { value, range }
    }

    /// Whether the number moved.
    pub fn show(self, ui: &mut Ui) -> bool {
        let Self { value, range } = self;
        ui.horizontal(|ui| {
            let slid = controls::slider(ui, value, range.clone()).changed();
            let typed = NumberField::new(value)
                .range(range)
                .width(SLIDER_FIELD_WIDTH)
                .show(ui);
            slid || typed
        })
        .inner
    }
}

/// Numbers typed in an [`InputField`], `, ` between them, taken on Enter or when the field loses
/// focus. One past the range is taken as the nearest it allows; text that doesn't read as the
/// numbers is dropped.
pub struct NumberField<'v, N, const K: usize> {
    values: [&'v mut N; K],
    range: Option<RangeInclusive<N>>,
    unit: Option<&'v str>,
    width: Option<f32>,
}

impl<'v, N> NumberField<'v, N, 1> {
    pub fn new(value: &'v mut N) -> Self {
        Self::of([value])
    }
}

impl<'v, N, const K: usize> NumberField<'v, N, K> {
    pub fn of(values: [&'v mut N; K]) -> Self {
        Self {
            values,
            range: None,
            unit: None,
            width: None,
        }
    }

    pub fn range(mut self, range: RangeInclusive<N>) -> Self {
        self.range = Some(range);
        self
    }

    pub fn unit(mut self, unit: &'v str) -> Self {
        self.unit = Some(unit);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl<N: Copy + PartialOrd + Display + FromStr, const K: usize> NumberField<'_, N, K> {
    /// Whether new numbers were taken.
    pub fn show(self, ui: &mut Ui) -> bool {
        let Self {
            values,
            range,
            unit,
            width,
        } = self;
        let id = ui.next_auto_id().with("number");
        let current = values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let typed = typed(ui, id, &current, |ui, draft| {
            let mut field = InputField::new(draft);
            if let Some(unit) = unit {
                field = field.unit(unit);
            }
            if let Some(width) = width {
                field = field.width(width);
            }
            field.show(ui)
        });
        let Typed::Committed(draft) = typed else {
            return false;
        };
        let Some(typed) = draft
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<N>()
                    .ok()
                    .and_then(|n| nearest(n, &range))
            })
            .collect::<Option<Vec<N>>>()
            .filter(|typed| typed.len() == K)
        else {
            return false;
        };
        let mut changed = false;
        for (value, typed) in values.into_iter().zip(typed) {
            if *value != typed {
                *value = typed;
                changed = true;
            }
        }
        changed
    }
}

/// A search: the field the narrowing of a list is typed in, `key` the shortcut that reaches it.
pub fn search(ui: &mut Ui, text: &mut String, hint: &str, key: Option<&str>) -> Response {
    boxed(ui, None, |ui| {
        crate::glyph::show(ui, crate::glyph::Mark::Search, 13.0, theme::MUTED);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(key) = key {
                controls::key_hint(ui, key);
            }
            ui.add(
                unframed(TextEdit::singleline(text))
                    .hint_text(theme::muted(hint))
                    .desired_width(ui.available_width()),
            )
        })
        .inner
    })
}

/// The input box a field sits in, beside its `unit`: an [`InputField`]'s, or one framing a field
/// of its own (a search, a line of styled text, a value shown where it can't be typed).
pub fn boxed(ui: &mut Ui, unit: Option<&str>, field: impl FnOnce(&mut Ui) -> Response) -> Response {
    let focused_id = ui.ctx().memory(|m| m.focused());
    let mut inner = None;
    let outer = frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(unit) = unit {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::faint(unit).size(12.0));
                    inner = Some(field(ui));
                });
            } else {
                inner = Some(field(ui));
            }
        });
    });
    let response = inner.expect("the field was drawn");
    if focused_id == Some(response.id) || response.has_focus() {
        controls::focus_ring(ui.painter(), outer.response.rect, controls::RADIUS);
    }
    response
}

/// The input box's look, for what sits among fields without being typed in (a pick, a key being
/// captured); its contents keep to one row of [`row_height`].
pub fn frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(controls::RADIUS)
        .inner_margin(egui::Margin::symmetric(8, 3))
}

pub fn row_height(ui: &Ui) -> f32 {
    ui.spacing().interact_size.y
}

/// `edit` as fields type in the input box, for a field [`boxed`] frames itself.
pub fn unframed(edit: TextEdit<'_>) -> TextEdit<'_> {
    edit.frame(egui::Frame::NONE)
        .font(egui::TextStyle::Body)
        .desired_width(f32::INFINITY)
}

/// The font fields type in, for a field that lays its text out itself.
pub fn font(ui: &Ui) -> egui::FontId {
    egui::TextStyle::Body.resolve(ui.style())
}

/// Whether a field was left without [`keys::CANCEL`], taking what was typed.
pub fn committed(ui: &Ui, response: &Response) -> bool {
    response.lost_focus() && !ui.input(|i| keys::pressed(i, keys::CANCEL))
}

/// Focus on `response` when nothing else has it, so a dialog opens typing.
pub fn default_focus(ui: &Ui, response: &Response) {
    if ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
}

fn within(ui: &mut Ui, width: Option<f32>, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    match width {
        Some(width) => {
            ui.scope(|ui| {
                ui.set_max_width(width);
                add(ui)
            })
            .inner
        }
        None => add(ui),
    }
}

fn nearest<N: Copy + PartialOrd>(number: N, range: &Option<RangeInclusive<N>>) -> Option<N> {
    number.partial_cmp(&number)?;
    Some(match range {
        Some(range) if number < *range.start() => *range.start(),
        Some(range) if number > *range.end() => *range.end(),
        _ => number,
    })
}

const SLIDER_FIELD_WIDTH: f32 = 64.0;
