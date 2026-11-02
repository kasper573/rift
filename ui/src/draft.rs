//! Text drafts: what a field is typed into while it has focus, taken or dropped when it lets go.

use egui::{Id, Response, Ui};

use crate::keys;

/// Where a field typed into with a draft stands this frame.
pub enum Typed {
    Pending,
    Committed(String),
    Cancelled,
}

/// A field of `current` typed into a draft of its own, kept under `id` while it has focus, and
/// taken when it loses focus unless the field was left with [`keys::CANCEL`].
pub fn typed(
    ui: &mut Ui,
    id: Id,
    current: &str,
    input: impl FnOnce(&mut Ui, &mut String) -> Response,
) -> Typed {
    let mut draft = draft_of(ui, id).unwrap_or_else(|| current.to_owned());
    let response = input(ui, &mut draft);
    let cancelled = ui.input(|i| keys::pressed(i, keys::CANCEL));
    if response.has_focus() && !cancelled {
        set_draft(ui, id, &draft);
        return Typed::Pending;
    }
    clear_draft(ui, id);
    if cancelled && (response.has_focus() || response.lost_focus()) {
        Typed::Cancelled
    } else if response.lost_focus() && draft != current {
        Typed::Committed(draft)
    } else {
        Typed::Pending
    }
}

/// The text a drafted field of `current` was committed to, when it changed.
pub fn drafted(
    ui: &mut Ui,
    id: Id,
    current: &str,
    input: impl FnOnce(&mut Ui, &mut String) -> Response,
) -> Option<String> {
    match typed(ui, id, current, input) {
        Typed::Committed(text) => Some(text),
        Typed::Pending | Typed::Cancelled => None,
    }
}

pub fn draft_of(ui: &Ui, id: Id) -> Option<String> {
    ui.ctx().data(|d| d.get_temp::<String>(id))
}

pub fn set_draft(ui: &Ui, id: Id, text: &str) {
    ui.ctx().data_mut(|d| d.insert_temp(id, text.to_owned()));
}

pub fn clear_draft(ui: &Ui, id: Id) {
    ui.ctx().data_mut(|d| d.remove_temp::<String>(id));
}
