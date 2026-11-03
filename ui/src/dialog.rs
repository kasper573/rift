//! Dialogs: a note laid over the interface that asks, tells or waits, its title on top and its
//! answers at the foot.

use egui::{Context, Event, Id, Margin, Order, Sense, Ui, Vec2};

use crate::face::TextRole;
use crate::{controls, ink, keys, paint, sheet};

/// The room between a dialog's edge and what it says.
pub const PADDING: i8 = 22;

pub fn title(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(TextRole::NoteTitle.rich(text).color(ink::INK)).wrap());
    ui.add_space(6.0);
}

/// A dialog's note `width` wide, laid where the caller has room for it.
pub fn note<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    sheet::note(ui, Margin::same(PADDING), |ui| {
        ui.set_width(width);
        add(ui)
    })
    .inner
}

/// A dialog's note in the middle of the screen.
pub fn card(gui: &Context, id: &str, width: f32, add: impl FnOnce(&mut Ui)) {
    egui::Area::new(Id::new(id))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .order(egui::Order::Middle)
        .show(gui, |ui| note(ui, width, add));
}

/// A dialog in the middle of the screen that can be resized, for one that holds a lot.
pub fn window(gui: &Context, text: &str, add: impl FnOnce(&mut Ui)) {
    let mut under = None;
    let shown = egui::Window::new(text)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .frame(egui::Frame::new().inner_margin(PADDING))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(gui, |ui| {
            under = Some(ui.painter().add(egui::Shape::Noop));
            title(ui, text);
            add(ui);
        });
    if let Some((shown, under)) = shown.zip(under) {
        let painter = gui.layer_painter(shown.response.layer_id);
        sheet::paint_note(
            &painter,
            under,
            shown.response.rect,
            sheet::shadow(sheet::Depth::Front),
        );
    }
}

/// The keys that answer a dialog, pressed this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub confirm: bool,
    pub cancel: bool,
}

pub fn keys(ui: &Ui) -> Keys {
    ui.ctx().input(|i| Keys {
        confirm: keys::pressed(i, keys::CONFIRM),
        cancel: keys::pressed(i, keys::CANCEL),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Confirm,
    Cancel,
}

/// The foot of a dialog: `confirm` and Cancel, each answered by its key as well.
pub fn answer_row(ui: &mut Ui, confirm: &str) -> Option<Answer> {
    let pressed = keys(ui);
    let mut answer = None;
    ui.horizontal(|ui| {
        if controls::primary(ui, confirm).clicked() || pressed.confirm {
            answer = Some(Answer::Confirm);
        }
        if controls::button(ui, "Cancel").clicked() || pressed.cancel {
            answer = Some(Answer::Cancel);
        }
    });
    answer
}

/// Whether a dialog was answered this frame, its keys before the `clicked` answer.
pub fn answer(clicked: Option<bool>, pressed: &Keys) -> Option<bool> {
    if pressed.cancel {
        Some(false)
    } else if pressed.confirm {
        Some(true)
    } else {
        clicked
    }
}

/// What a dialog that waits says: its `title`, what is being done, and how far that has come when
/// it is known.
pub struct Waiting<'a> {
    pub title: &'a str,
    pub line: String,
    pub fraction: Option<f32>,
}

/// A veil over the whole interface, taking every input meant for what is under it: its clicks,
/// drags and keys this frame, and the focus of a field being typed into. With `waiting` it dims
/// the interface and says what it waits on. Laid before anything else is drawn, so nothing under
/// it reads the keys it takes.
pub fn veil(gui: &Context, id: &str, waiting: Option<&Waiting>) {
    gui.memory_mut(|memory| memory.stop_text_input());
    gui.input_mut(|input| input.events.retain(|event| !reaches_widgets(event)));
    let screen = gui.viewport_rect();
    egui::Area::new(Id::new(id))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(gui, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            if waiting.is_some() {
                ui.painter()
                    .rect_filled(rect, 0.0, ink::wash(ink::VEIL, VEIL_DIM));
            }
        });
    let Some(waiting) = waiting else {
        return;
    };
    // Above the veil, which comes to the top of its layer whenever it is clicked.
    egui::Area::new(Id::new(id).with("waiting"))
        .order(Order::Tooltip)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .interactable(false)
        .show(gui, |ui| {
            note(ui, WAITING_WIDTH, |ui| {
                title(ui, waiting.title);
                ui.add(
                    egui::Label::new(TextRole::Body.rich(&waiting.line).color(ink::INK2))
                        .truncate(),
                );
                ui.add_space(10.0);
                let size = Vec2::new(ui.available_width(), WAITING_BAR);
                match waiting.fraction {
                    Some(fraction) => paint::progress_line(ui, size, fraction, ink::INK2),
                    None => sweep(ui, size),
                }
            });
        });
}

/// How much the veil darkens the interface under a dialog that waits.
const VEIL_DIM: f32 = 0.35;
const WAITING_WIDTH: f32 = 440.0;
const WAITING_BAR: f32 = 3.0;
/// How long the sweep of a wait of unknown length takes to cross its track and back.
const SWEEP_SECONDS: f64 = 1.6;
/// How much of its track the sweep covers.
const SWEEP_SHARE: f32 = 0.25;

/// Whether `event` is input a widget acts on, as opposed to what the window tells of itself.
fn reaches_widgets(event: &Event) -> bool {
    matches!(
        event,
        Event::Key { .. }
            | Event::Text(_)
            | Event::Paste(_)
            | Event::Copy
            | Event::Cut
            | Event::Ime(_)
    )
}

/// A track with a stretch of it sweeping back and forth: a wait whose length isn't known.
fn sweep(ui: &mut Ui, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let phase = (ui.input(|input| input.time) / SWEEP_SECONDS).fract() as f32;
    let there_and_back = 1.0 - (2.0 * phase - 1.0).abs();
    let width = rect.width() * SWEEP_SHARE;
    let left = rect.min.x + (rect.width() - width) * there_and_back;
    let y = rect.center().y;
    let painter = ui.painter();
    painter.hline(
        rect.x_range(),
        y,
        egui::Stroke::new(1.0, ink::ink_wash(0.4)),
    );
    painter.hline(left..=left + width, y, egui::Stroke::new(size.y, ink::INK2));
    ui.ctx().request_repaint();
}
