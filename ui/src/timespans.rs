//! Named spans that divide a range between them, edited on one track: where each begins, how
//! softly it comes in, and how it is shown. The last runs round the range's end into the first.

use std::ops::RangeInclusive;

use egui::{Color32, DragValue, Id, Rect, Sense, Stroke, TextEdit, TextureId, Ui, Vec2};

use crate::{controls, draft, ink, paint, theme};

/// A span of a range that its fellows divide between them: where it begins, how softly, and how
/// it is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Timespan {
    pub name: String,
    pub from: f32,
    pub eases: f32,
    pub color: Color32,
    pub icon: Option<TextureId>,
}

/// What an edit of the spans did, for the caller to write into whatever the spans stand for.
#[derive(Clone, Debug, PartialEq)]
pub enum TimespanEdit {
    Added { at: usize, from: f32 },
    Removed { at: usize },
    Renamed { at: usize, name: String },
    Moved { at: usize, from: f32 },
    Eased { at: usize, eases: f32 },
    Coloured { at: usize, color: Color32 },
    PickIcon { at: usize },
}

/// Draws `spans` over `range`, wrapping round its end, and yields what the user did.
pub fn timespans(
    ui: &mut Ui,
    range: RangeInclusive<f32>,
    spans: &[Timespan],
) -> Option<TimespanEdit> {
    let mut edit = None;
    let (start, length) = (*range.start(), range.end() - range.start());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), TRACK), Sense::click());
    let x_of = |value: f32| rect.left() + (value - start) / length * rect.width();
    let value_at = |x: f32| snapped(start + (x - rect.left()) / rect.width() * length);
    let mut picked = picked(ui).filter(|&at| at < spans.len());

    paint::band(ui.painter(), rect, |along| {
        color_at(spans, &range, start + along * length)
    });
    for (at, span) in spans.iter().enumerate() {
        for piece in pieces(spans, at, &range) {
            let piece =
                Rect::from_x_y_ranges(x_of(*piece.start())..=x_of(*piece.end()), rect.y_range());
            name_piece(ui, piece, span, picked == Some(at));
        }
    }
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    if let Some(pointer) = response.interact_pointer_pos() {
        let value = value_at(pointer.x);
        if response.double_clicked() {
            let at = spans
                .iter()
                .position(|span| span.from > value)
                .unwrap_or(spans.len());
            edit = Some(TimespanEdit::Added { at, from: value });
        } else if response.clicked() {
            picked = holding(spans, value);
        }
    }
    for (at, span) in spans.iter().enumerate() {
        let x = x_of(span.from);
        let grip = Rect::from_center_size(
            egui::pos2(x, rect.center().y),
            Vec2::new(GRIP, rect.height() + 6.0),
        );
        let dragged = ui
            .interact(grip, ui.id().with(("timespan", at)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if let Some(pointer) = dragged.interact_pointer_pos().filter(|_| dragged.dragged()) {
            let (low, high) = room(spans, at, &range);
            let from = value_at(pointer.x).clamp(low, high);
            if from != span.from {
                edit = Some(TimespanEdit::Moved { at, from });
            }
            picked = Some(at);
        }
        let ink = if dragged.hovered() || dragged.dragged() {
            ink::RUBRIC
        } else {
            ink::INK
        };
        ui.painter().vline(x, grip.y_range(), Stroke::new(2.0, ink));
    }

    if let Some(at) = picked {
        edit = edit.or(fields(ui, &range, spans, at));
    }
    ui.ctx().data_mut(|data| match picked {
        Some(at) => {
            data.insert_temp(picked_id(), at);
        }
        None => {
            data.remove_temp::<usize>(picked_id());
        }
    });
    edit
}

/// Which of the spans is picked on the track, if any.
pub fn picked(ui: &Ui) -> Option<usize> {
    ui.ctx().data(|data| data.get_temp::<usize>(picked_id()))
}

/// The colour the spans give `value` of `range`: each span's own, blending into the next over
/// that one's `eases`, half of it before the next begins and half after.
pub fn color_at(spans: &[Timespan], range: &RangeInclusive<f32>, value: f32) -> Color32 {
    let Some(at) = holding(spans, value) else {
        return theme::RAISED;
    };
    let length = range.end() - range.start();
    let apart = |a: f32, b: f32| (a - b).rem_euclid(length);
    let (before, here, after) = (
        &spans[(at + spans.len() - 1) % spans.len()],
        &spans[at],
        &spans[(at + 1) % spans.len()],
    );
    let (since, until) = (apart(value, here.from), apart(after.from, value));
    if spans.len() == 1 {
        here.color
    } else if since < here.eases / 2.0 {
        blend(before.color, here.color, 0.5 + since / here.eases)
    } else if until < after.eases / 2.0 {
        blend(here.color, after.color, 0.5 - until / after.eases)
    } else {
        here.color
    }
}

const TRACK: f32 = 34.0;
const GRIP: f32 = 9.0;
const RADIUS: f32 = 3.0;
const ICON: f32 = 16.0;
const NAME_WIDTH: f32 = 110.0;
/// How many steps of a unit of the range a span begins and eases on.
const STEPS: f32 = 4.0;

fn picked_id() -> Id {
    Id::new("ui::timespans::picked")
}

fn snapped(value: f32) -> f32 {
    (value * STEPS).round() / STEPS
}

/// The span that holds `value`: the last to begin at or before it, or else the last of all,
/// which runs round the range's end.
fn holding(spans: &[Timespan], value: f32) -> Option<usize> {
    spans
        .iter()
        .rposition(|span| span.from <= value)
        .or(spans.len().checked_sub(1))
}

/// The stretches of the range the span at `at` holds: one, or two when it runs round the end.
fn pieces(spans: &[Timespan], at: usize, range: &RangeInclusive<f32>) -> Vec<RangeInclusive<f32>> {
    let (from, to) = (spans[at].from, spans[(at + 1) % spans.len()].from);
    if spans.len() == 1 {
        vec![range.clone()]
    } else if from < to {
        vec![from..=to]
    } else {
        vec![from..=*range.end(), *range.start()..=to]
    }
}

/// Between which values the span at `at` may begin: a step clear of the spans beside it, and
/// never past either end of the range, so the spans keep their order.
fn room(spans: &[Timespan], at: usize, range: &RangeInclusive<f32>) -> (f32, f32) {
    let step = 1.0 / STEPS;
    let low = match at.checked_sub(1) {
        Some(before) => spans[before].from + step,
        None => *range.start(),
    };
    let high = match spans.get(at + 1) {
        Some(after) => after.from - step,
        None => range.end() - step,
    };
    (low, high.max(low))
}

/// How long a span at `at` may ease in at the most: no longer than it lasts, or than the one
/// before it does.
fn longest_ease(spans: &[Timespan], at: usize, range: &RangeInclusive<f32>) -> f32 {
    let length = range.end() - range.start();
    if spans.len() == 1 {
        return length;
    }
    let lasts =
        |at: usize| (spans[(at + 1) % spans.len()].from - spans[at].from).rem_euclid(length);
    lasts(at).min(lasts((at + spans.len() - 1) % spans.len()))
}

fn name_piece(ui: &Ui, piece: Rect, span: &Timespan, picked: bool) {
    let painter = ui.painter().with_clip_rect(piece);
    let ink = readable_on(span.color);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = painter.layout_no_wrap(span.name.clone(), font, ink);
    let icon = span.icon.map_or(0.0, |_| ICON + 4.0);
    let width = icon + galley.size().x;
    if width + 8.0 > piece.width() {
        return;
    }
    let left = piece.center().x - width / 2.0;
    if let Some(icon) = span.icon {
        let at = Rect::from_min_size(
            egui::pos2(left, piece.center().y - ICON / 2.0),
            Vec2::splat(ICON),
        );
        egui::Image::new((icon, at.size())).paint_at(ui, at);
    }
    let at = egui::pos2(left + icon, piece.center().y - galley.size().y / 2.0);
    painter.galley(at, galley, ink);
    if picked {
        painter.rect_stroke(
            piece.shrink(2.0),
            RADIUS,
            Stroke::new(1.5, ink),
            egui::StrokeKind::Inside,
        );
    }
}

fn fields(
    ui: &mut Ui,
    range: &RangeInclusive<f32>,
    spans: &[Timespan],
    at: usize,
) -> Option<TimespanEdit> {
    let span = &spans[at];
    let mut edit = None;
    ui.horizontal(|ui| {
        let renamed = draft::drafted(ui, ui.id().with(("timespan name", at)), &span.name, {
            |ui, text| ui.add(TextEdit::singleline(text).desired_width(NAME_WIDTH))
        });
        if let Some(name) = renamed.filter(|name| !name.trim().is_empty()) {
            edit = Some(TimespanEdit::Renamed { at, name });
        }
        let (low, high) = room(spans, at, range);
        let mut from = span.from;
        ui.label(theme::muted("from"));
        if ui
            .add(
                DragValue::new(&mut from)
                    .speed(1.0 / STEPS)
                    .range(low..=high),
            )
            .changed()
        {
            edit = Some(TimespanEdit::Moved {
                at,
                from: snapped(from),
            });
        }
        let mut eases = span.eases;
        ui.label(theme::muted("eases in over"));
        if ui
            .add(
                DragValue::new(&mut eases)
                    .speed(1.0 / STEPS)
                    .range(0.0..=longest_ease(spans, at, range)),
            )
            .changed()
        {
            edit = Some(TimespanEdit::Eased {
                at,
                eases: snapped(eases),
            });
        }
        let mut color = span.color;
        if ui.color_edit_button_srgba(&mut color).changed() {
            edit = Some(TimespanEdit::Coloured { at, color });
        }
        let icon = match span.icon {
            Some(icon) => ui
                .add(egui::Button::image(egui::Image::new((icon, Vec2::splat(ICON)))).frame(false)),
            None => controls::button(ui, "Icon…"),
        };
        if icon.on_hover_text("Pick its icon").clicked() {
            edit = Some(TimespanEdit::PickIcon { at });
        }
        if spans.len() > 1
            && controls::remove(ui, "Remove it: its stretch goes to the one before").clicked()
        {
            edit = Some(TimespanEdit::Removed { at });
        }
    });
    edit
}

fn blend(from: Color32, to: Color32, along: f32) -> Color32 {
    let along = along.clamp(0.0, 1.0);
    let channel = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * along).round() as u8
    };
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

/// Dark ink on a light colour and light ink on a dark one.
fn readable_on(color: Color32) -> Color32 {
    let light =
        0.299 * f32::from(color.r()) + 0.587 * f32::from(color.g()) + 0.114 * f32::from(color.b());
    if light > 140.0 { ink::INK } else { ink::LIGHT }
}
