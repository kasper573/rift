//! Menus: a button that opens one, an entry that opens another beside it, one at the pointer.

use egui::{InnerResponse, Rect, Response, RichText, Ui, Vec2, WidgetText};

use crate::glyph::Mark;
use crate::theme;

/// A plain button that opens a menu, its caret on the right.
pub fn button<R>(
    ui: &mut Ui,
    text: impl Into<RichText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let button = egui::Button::new(text.into()).right_text(Mark::CaretDown.text(11.0));
    let (response, inner) =
        egui::containers::menu::MenuButton::from_button(button).ui(ui, contents);
    InnerResponse::new(inner.map(|inner| inner.inner), response)
}

/// A button in a menu that opens `content` beside it, marked with the interface's own chevron.
pub fn submenu<R>(
    ui: &mut Ui,
    text: impl Into<WidgetText>,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let button = egui::Button::new(text.into()).right_text("   ");
    let (response, inner) =
        egui::containers::menu::SubMenuButton::from_button(button).ui(ui, content);
    let at = response.rect.right_center() - Vec2::new(10.0, 0.0);
    Mark::CaretRight.paint(
        ui.painter(),
        Rect::from_center_size(at, Vec2::splat(9.0)),
        theme::MUTED,
    );
    inner.map(|inner| inner.inner)
}

/// A menu at the pointer, laid out as a right-click menu is, but opened whenever `open` says.
pub fn at_pointer<R>(response: &Response, open: bool, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let command = if open {
        Some(egui::SetOpenCommand::Bool(true))
    } else if response.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    egui::Popup::menu(response)
        .open_memory(command)
        .at_pointer_fixed()
        .show(add)
        .map(|shown| shown.inner)
}

/// An open menu's "Move up" and "Move down" entries for the entry at `index` of `count`: the two
/// indices to swap, once one is picked.
pub fn move_entries(ui: &mut Ui, index: usize, count: usize) -> Option<(usize, usize)> {
    let mut swap = None;
    if index > 0 && ui.button("Move up").clicked() {
        swap = Some((index, index - 1));
    }
    if index + 1 < count && ui.button("Move down").clicked() {
        swap = Some((index, index + 1));
    }
    if swap.is_some() {
        ui.close();
    }
    swap
}

/// Whether what `id` names shows this pass without having shown the pass before, so each opening
/// can start afresh however the last one closed.
pub fn opened_afresh(ui: &Ui, id: egui::Id) -> bool {
    let pass = ui.ctx().cumulative_pass_nr();
    let shown_id = id.with("shown");
    let last = ui.ctx().data(|d| d.get_temp::<u64>(shown_id));
    ui.ctx().data_mut(|d| d.insert_temp(shown_id, pass));
    last.is_none_or(|last| last + 1 < pass)
}
