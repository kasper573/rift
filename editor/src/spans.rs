use std::collections::HashMap;
use ui::chip;
use ui::controls;
use ui::egui;
use ui::form;
use ui::menu;
use ui::theme;

use engine::core::script::Expr;
use engine::core::text::{TextMotion, TextVoice};
use engine::core::time::Millis;
use engine::systems::input::map::InputAction;
use engine::systems::text::{Fx, Ink, Span};
use strum::VariantArray;
use ui::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use ui::egui::{Color32, FontId, RichText, Stroke, Ui};
use ui::ink::{self, Ground};

use crate::forms::{self, FormChange, FormCx};
use crate::keymap::EditorKey;
use ui::input;
use ui::input::NumberField;
use ui::input::committed;

const VALUE_TOKEN: &str = "⟨value⟩";
const INPUT_TOKEN: &str = "⟨input⟩";
const PAUSE: Millis = Millis(400.0);
const PAUSE_WIDTH: f32 = 96.0;

pub struct SpanLineEdit {
    pub spans: Option<Vec<Span>>,
    pub entered: bool,
    pub response: egui::Response,
}

pub fn line(ui: &mut Ui, stored: &[Span], id: egui::Id, font: FontId, hint: &str) -> SpanLineEdit {
    let draft: Option<Vec<Span>> = ui.ctx().data(|d| d.get_temp(id));
    let mut dirty = draft.is_some();
    let mut spans_now = draft.unwrap_or_else(|| stored.to_vec());
    let plain_before = plain(&spans_now);
    let mut text = plain_before.clone();
    let selection_id = id.with("selection");
    let selection: Option<(usize, usize)> = ui.ctx().data(|d| d.get_temp(selection_id));
    if let Some((start, end)) = selection.filter(|(a, b)| a != b)
        && let Some(styled) = toolbar(ui, &spans_now, (start, end))
    {
        spans_now = styled;
        dirty = true;
    }
    let job_spans = spans_now.clone();
    let font_for_layout = font.clone();
    let mut layouter = move |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let typed = buffer.as_str();
        let job = if typed == plain(&job_spans) {
            layout_job(&job_spans, font_for_layout.clone(), theme::TEXT, wrap_width)
        } else {
            let mut job = LayoutJob::simple(
                typed.to_owned(),
                font_for_layout.clone(),
                theme::TEXT,
                wrap_width,
            );
            job.wrap.max_width = wrap_width;
            job
        };
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = egui::TextEdit::multiline(&mut text)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(hint)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter)
        .show(ui);
    let response = output.response;
    let focused = response.has_focus();
    let range = output
        .state
        .cursor
        .char_range()
        .filter(|_| focused)
        .map(|range| {
            let sorted = range.sorted_cursors();
            (sorted[0].index, sorted[1].index)
        });
    ui.ctx().data_mut(|d| match range {
        Some(range) => {
            d.insert_temp(selection_id, range);
        }
        None => {
            d.remove_temp::<(usize, usize)>(selection_id);
        }
    });
    let enter = ui.input(|i| EditorKey::Confirm.pressed(i));
    let mut entered = false;
    if response.changed() {
        let typed = text.replace('\n', "");
        if typed != text && enter {
            entered = true;
        }
        if let Some(retyped) = retype(&spans_now, &plain_before, &typed) {
            spans_now = retyped;
            dirty = true;
        } else {
            ui.ctx().request_repaint();
        }
    }
    if entered {
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(
                plain(&spans_now).chars().count(),
            ))));
        state.store(ui.ctx(), response.id);
    }
    if dirty {
        ui.ctx().data_mut(|d| d.insert_temp(id, spans_now.clone()));
    }
    let forget = |ui: &Ui| ui.ctx().data_mut(|d| d.remove_temp::<Vec<Span>>(id));
    let escape = ui.input(|i| EditorKey::Cancel.pressed(i));
    if escape && focused {
        forget(ui);
        return SpanLineEdit {
            spans: None,
            entered: false,
            response: response.response,
        };
    }
    if entered || committed(ui, &response) {
        forget(ui);
        if dirty {
            return SpanLineEdit {
                spans: Some(spans_now),
                entered,
                response: response.response,
            };
        }
    }
    SpanLineEdit {
        spans: None,
        entered,
        response: response.response,
    }
}

pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(token).collect()
}

pub fn layout_job(spans: &[Span], font: FontId, ink: Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    for span in spans {
        let mut format = TextFormat::simple(font.clone(), ink);
        match span {
            Span::Value { .. } | Span::Input(_) => {
                format.color = theme::CODE;
                format.background = theme::INPUT;
            }
            Span::Text { fx, .. } => {
                for fx in fx {
                    match fx {
                        Fx::Ink(ink) => format.color = ink.prose().on(Ground::Paper),
                        Fx::Voice(_) | Fx::Motion(_) => format.background = ink::gold_wash(0.2),
                        Fx::Slow => format.underline = Stroke::new(1.0, format.color),
                        Fx::PauseAfter(_) => {
                            format.strikethrough = Stroke::NONE;
                            format.italics = true;
                        }
                    }
                }
            }
        }
        job.append(&token(span), 0.0, format);
    }
    job
}

pub fn retype(spans: &[Span], before: &str, after: &str) -> Option<Vec<Span>> {
    let old: Vec<char> = before.chars().collect();
    let new: Vec<char> = after.chars().collect();
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut cells = Cells::of(spans);
    let removed = prefix..old.len() - suffix;
    if cells.cuts_atom(removed.start) || cells.cuts_atom(removed.end) {
        return None;
    }
    let run = cells.run_at(removed.start);
    let inserted: Vec<Cell> = new[prefix..new.len() - suffix]
        .iter()
        .map(|&c| Cell::Char(c, run))
        .collect();
    cells.cells.splice(removed, inserted);
    Some(cells.spans())
}

fn toolbar(ui: &mut Ui, spans: &[Span], range: (usize, usize)) -> Option<Vec<Span>> {
    let mut outcome = None;
    chip::holding(ui, theme::RAISED, theme::BORDER, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut styled = |kind: FxKind, to: Option<Fx>| {
            outcome = Some(restyle(spans, range, |fx| kind.set(fx, to)));
        };
        fx_menu(ui, FxKind::Ink, Ink::VARIANTS, Fx::Ink, &mut styled);
        fx_menu(
            ui,
            FxKind::Voice,
            TextVoice::VARIANTS,
            Fx::Voice,
            &mut styled,
        );
        fx_menu(
            ui,
            FxKind::Motion,
            TextMotion::VARIANTS,
            Fx::Motion,
            &mut styled,
        );
        if ui
            .add(egui::Button::new(RichText::new("slow").size(12.0)))
            .clicked()
        {
            outcome = Some(restyle(spans, range, |fx| {
                FxKind::Slow.toggle(fx, Fx::Slow)
            }));
        }
        if ui
            .add(egui::Button::new(RichText::new("pause").size(12.0)))
            .on_hover_text("A 400 ms pause after the span")
            .clicked()
        {
            outcome = Some(restyle(spans, range, |fx| {
                FxKind::PauseAfter.toggle(fx, Fx::PauseAfter(PAUSE))
            }));
        }
        if ui
            .add(egui::Button::new(RichText::new("value").size(12.0)))
            .on_hover_text("Replace the selection with a value span showing an expression")
            .clicked()
        {
            outcome = with_value(spans, range);
        }
        ui.label(theme::faint(format!("{} chars", range.1 - range.0)).size(11.0));
    });
    outcome
}

fn fx_menu<T: Copy + Into<&'static str>>(
    ui: &mut Ui,
    kind: FxKind,
    options: &[T],
    wrap: fn(T) -> Fx,
    styled: &mut impl FnMut(FxKind, Option<Fx>),
) {
    let field = kind.name();
    menu::button(ui, RichText::new(field).size(12.0), |ui| {
        for &option in options {
            if ui.button(option.into()).clicked() {
                styled(kind, Some(wrap(option)));
                ui.close();
            }
        }
        if ui.button(format!("no {field}")).clicked() {
            styled(kind, None);
            ui.close();
        }
    });
}

/// The line with the selection between `start` and `end` replaced by a value span.
pub fn with_value(spans: &[Span], (start, end): (usize, usize)) -> Option<Vec<Span>> {
    let mut cells = Cells::of(spans);
    if cells.cuts_atom(start) || cells.cuts_atom(end) {
        return None;
    }
    cells.cells.drain(start..end);
    cells.insert_atom(
        start,
        Span::Value {
            value: Box::new(Expr::Int(0)),
            fx: Vec::new(),
        },
    );
    Some(cells.spans())
}

pub fn restyle(
    spans: &[Span],
    (start, end): (usize, usize),
    apply: impl Fn(&mut Vec<Fx>),
) -> Vec<Span> {
    let Cells {
        mut runs,
        mut cells,
    } = Cells::of(spans);
    let mut restyled: HashMap<usize, usize> = HashMap::new();
    let end = end.min(cells.len());
    for cell in &mut cells[start.min(end)..end] {
        if let Cell::Char(_, run) = cell {
            *run = *restyled.entry(*run).or_insert_with(|| {
                let mut style = runs[*run].clone();
                apply(&mut style);
                runs.push(style);
                runs.len() - 1
            });
        }
    }
    Cells { runs, cells }.spans()
}

/// One span as a form edits it: its text or what it shows, and its effects.
pub fn edit(ui: &mut Ui, cx: &mut FormCx, span: &mut Span, path: &str) -> FormChange {
    let mut changed = FormChange::default();
    ui.vertical(|ui| match span {
        Span::Input(action) => {
            ui.horizontal(|ui| {
                ui.label("key of");
                let mut picked = *action;
                controls::combo(ui.id().with(path).with("input"))
                    .selected_text(<&str>::from(*action))
                    .show_ui(ui, |ui| {
                        for &option in InputAction::VARIANTS {
                            ui.selectable_value(&mut picked, option, <&str>::from(option));
                        }
                    });
                if picked != *action {
                    *action = picked;
                    changed = FormChange::at(path);
                }
            });
        }
        Span::Value { value, fx } => {
            ui.label(RichText::new("value").weak());
            changed |= forms::of_type(ui, cx, &mut **value, &format!("{path}/value"));
            changed |= effects(ui, fx, path);
        }
        Span::Text { text, fx } => {
            changed |= forms::text_of(ui, cx, text, &format!("{path}/text"));
            changed |= effects(ui, fx, path);
        }
    });
    changed
}

/// A line of spans as a form edits it: typed in place, with the expression of each value span
/// below it.
pub(crate) fn line_field(
    ui: &mut Ui,
    cx: &mut FormCx,
    spans: &mut Vec<Span>,
    path: &str,
) -> FormChange {
    let id = ui.id().with(path).with("line");
    let mut changed = FormChange::default();
    input::boxed(ui, None, |ui| {
        let edit = line(ui, spans, id, input::font(ui), "…");
        if let Some(typed) = edit.spans {
            *spans = typed;
            changed = FormChange::at(path);
        }
        edit.response
    });
    for (index, span) in spans.iter_mut().enumerate() {
        let Span::Value { value, .. } = span else {
            continue;
        };
        form::kv(ui, "value", |ui| {
            changed |= forms::of_type(ui, cx, &mut **value, &format!("{path}/{index}/value"));
        });
    }
    changed
}

fn effects(ui: &mut Ui, fx: &mut Vec<Fx>, path: &str) -> FormChange {
    let mut changed = FormChange::default();
    ui.horizontal_wrapped(|ui| {
        changed |= fx_combo(ui, fx, path, FxKind::Ink, Ink::VARIANTS, Fx::Ink);
        changed |= fx_combo(ui, fx, path, FxKind::Voice, TextVoice::VARIANTS, Fx::Voice);
        changed |= fx_combo(
            ui,
            fx,
            path,
            FxKind::Motion,
            TextMotion::VARIANTS,
            Fx::Motion,
        );
        let mut slow = FxKind::Slow.of(fx).is_some();
        if ui.checkbox(&mut slow, "slow").changed() {
            FxKind::Slow.set(fx, slow.then_some(Fx::Slow));
            changed = FormChange::at(path);
        }
        let mut pause = match FxKind::PauseAfter.of(fx) {
            Some(Fx::PauseAfter(pause)) => pause.0,
            _ => 0.0,
        };
        ui.label("pause");
        if NumberField::new(&mut pause)
            .range(0.0..=10_000.0)
            .unit("ms")
            .width(PAUSE_WIDTH)
            .show(ui)
        {
            let pause = (pause > 0.0).then_some(Fx::PauseAfter(Millis(pause)));
            FxKind::PauseAfter.set(fx, pause);
            changed = FormChange::at(path);
        }
    });
    changed
}

fn fx_combo<T: Copy + PartialEq + Into<&'static str>>(
    ui: &mut Ui,
    fx: &mut Vec<Fx>,
    path: &str,
    kind: FxKind,
    options: &[T],
    wrap: fn(T) -> Fx,
) -> FormChange {
    let field = kind.name();
    let current = options
        .iter()
        .copied()
        .find(|option| kind.of(fx) == Some(wrap(*option)));
    let mut picked = current;
    controls::combo(ui.id().with(path).with(field))
        .selected_text(current.map_or(field, Into::into))
        .width(90.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut picked, None, format!("no {field}"));
            for &option in options {
                ui.selectable_value(&mut picked, Some(option), option.into());
            }
        });
    if picked == current {
        return FormChange::default();
    }
    kind.set(fx, picked.map(wrap));
    FormChange::at(path)
}

/// Which of a span's effects an effect is, each held at most once.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FxKind {
    Ink,
    Voice,
    Motion,
    Slow,
    PauseAfter,
}

impl FxKind {
    fn of_fx(fx: &Fx) -> FxKind {
        match fx {
            Fx::Ink(_) => FxKind::Ink,
            Fx::Voice(_) => FxKind::Voice,
            Fx::Motion(_) => FxKind::Motion,
            Fx::Slow => FxKind::Slow,
            Fx::PauseAfter(_) => FxKind::PauseAfter,
        }
    }

    fn name(self) -> &'static str {
        match self {
            FxKind::Ink => "ink",
            FxKind::Voice => "voice",
            FxKind::Motion => "motion",
            FxKind::Slow => "slow",
            FxKind::PauseAfter => "pause",
        }
    }

    fn of(self, fx: &[Fx]) -> Option<Fx> {
        fx.iter().copied().find(|held| FxKind::of_fx(held) == self)
    }

    fn set(self, fx: &mut Vec<Fx>, to: Option<Fx>) {
        fx.retain(|held| FxKind::of_fx(held) != self);
        fx.extend(to);
    }

    fn toggle(self, fx: &mut Vec<Fx>, on: Fx) {
        let to = self.of(fx).is_none().then_some(on);
        self.set(fx, to);
    }
}

fn token(span: &Span) -> String {
    match span {
        Span::Text { text, .. } => text.clone(),
        Span::Value { .. } => VALUE_TOKEN.to_owned(),
        Span::Input(_) => INPUT_TOKEN.to_owned(),
    }
}

/// A line cut into the characters its plain text shows: a character keeps the run of effects it
/// was written in, and a value or input span sits whole on the first cell of its token.
struct Cells {
    runs: Vec<Vec<Fx>>,
    cells: Vec<Cell>,
}

#[derive(Clone)]
enum Cell {
    Char(char, usize),
    Atom(Span),
    WithinAtom,
}

impl Cells {
    fn of(spans: &[Span]) -> Cells {
        let mut cells = Cells {
            runs: Vec::new(),
            cells: Vec::new(),
        };
        for span in spans {
            let Span::Text { text, fx } = span else {
                cells.insert_atom(cells.cells.len(), span.clone());
                continue;
            };
            cells.runs.push(fx.clone());
            let run = cells.runs.len() - 1;
            cells.cells.extend(text.chars().map(|c| Cell::Char(c, run)));
        }
        cells
    }

    fn insert_atom(&mut self, at: usize, span: Span) {
        let width = token(&span).chars().count();
        let cells = std::iter::once(Cell::Atom(span)).chain(std::iter::repeat_n(
            Cell::WithinAtom,
            width.saturating_sub(1),
        ));
        self.cells.splice(at..at, cells);
    }

    /// Whether a cut at `at` would split a value or input span.
    fn cuts_atom(&self, at: usize) -> bool {
        matches!(self.cells.get(at), Some(Cell::WithinAtom))
    }

    /// The run text typed at `at` joins: the one before it, else the one after, else plain text.
    fn run_at(&mut self, at: usize) -> usize {
        let neighbour = |index: Option<usize>| match index.and_then(|i| self.cells.get(i)) {
            Some(Cell::Char(_, run)) => Some(*run),
            _ => None,
        };
        neighbour(at.checked_sub(1))
            .or(neighbour(Some(at)))
            .unwrap_or_else(|| {
                self.runs.push(Vec::new());
                self.runs.len() - 1
            })
    }

    fn spans(&self) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        let mut run: Option<(usize, String)> = None;
        for cell in &self.cells {
            match cell {
                Cell::Char(c, of) => match &mut run {
                    Some((current, text)) if current == of => text.push(*c),
                    _ => {
                        spans.extend(run.take().map(|run| self.span_of(run)));
                        run = Some((*of, c.to_string()));
                    }
                },
                Cell::Atom(span) => {
                    spans.extend(run.take().map(|run| self.span_of(run)));
                    spans.push(span.clone());
                }
                Cell::WithinAtom => {}
            }
        }
        spans.extend(run.map(|run| self.span_of(run)));
        let mut merged: Vec<Span> = Vec::new();
        for span in spans {
            match (merged.last_mut(), span) {
                (
                    Some(Span::Text {
                        text: last,
                        fx: before,
                    }),
                    Span::Text { text: next, fx },
                ) if *before == fx => last.push_str(&next),
                (_, span) => merged.push(span),
            }
        }
        if merged.is_empty() {
            merged.push(engine::systems::text::plain(""));
        }
        merged
    }

    fn span_of(&self, (run, text): (usize, String)) -> Span {
        Span::Text {
            text,
            fx: self.runs[run].clone(),
        }
    }
}
