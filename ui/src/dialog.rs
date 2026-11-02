//! Dialogs: a note laid over the interface that asks, tells or waits, its title on top and its
//! answers at the foot.

use egui::{Context, Id, Margin, Ui};

use crate::face::TextRole;
use crate::{controls, ink, keys, sheet};

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
