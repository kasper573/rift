use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use engine::systems::input::map::{
    InputAction, InputBinding, InputBindingKind, InputDef, KeyModifiers,
};
use serde::Deserialize;
use serde::de::value::{Error as NameError, StrDeserializer};
use serde_json::Value;
use ui::chip;
use ui::controls;
use ui::egui;
use ui::egui::{Key, Sense, Ui};
use ui::glyph::{self, Mark};
use ui::input;
use ui::theme;

use super::{CellEdit, SpreadsheetRow};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::ContentEdit;

/// The bindings a cell holds, as the player reads them.
pub(super) fn names(value: &Value) -> String {
    let bindings: Vec<InputBinding> = serde_json::from_value(value.clone()).unwrap_or_default();
    match bindings.as_slice() {
        [] => "none".to_owned(),
        bindings => bindings
            .iter()
            .map(InputBinding::name)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The bindings of the action `row` is, each a chip, and a field that captures the next key or
/// click as one more, of a kind the game reads for the action. Writes as it goes; Enter closes it.
pub(super) fn cell(editor: &mut Editor, ui: &mut Ui, row: &SpreadsheetRow) -> CellEdit {
    let Ok(action) = InputAction::deserialize(StrDeserializer::<NameError>::new(&row.key)) else {
        return CellEdit::Cancelled;
    };
    let Some(Ok(input)) = editor.read::<InputDef>(&row.file) else {
        return CellEdit::Cancelled;
    };
    let mut input = (*input).clone();
    let capturing_id = ui.id().with("capture");
    let capturing = ui.ctx().data(|d| d.get_temp::<bool>(capturing_id)) == Some(true);
    if !capturing && ui.input(|i| EditorKey::Cancel.pressed(i)) {
        return CellEdit::Cancelled;
    }
    let changed = ui
        .vertical(|ui| bindings(ui, &mut input, action, capturing_id, capturing))
        .inner;
    if changed {
        let label = format!("Edit the bindings of {}", row.key);
        editor.write_content_as(ContentEdit::new(label), &row.file, &input);
    }
    if !capturing && ui.input(|i| EditorKey::Confirm.pressed(i)) {
        CellEdit::Done
    } else {
        CellEdit::Open
    }
}

fn bindings(
    ui: &mut Ui,
    input: &mut InputDef,
    action: InputAction,
    capturing_id: egui::Id,
    capturing: bool,
) -> bool {
    let reads: Vec<InputBindingKind> = action
        .readings()
        .iter()
        .flat_map(|gesture| gesture.bindings())
        .copied()
        .collect();
    let others: Vec<(InputAction, Vec<InputBinding>)> = input
        .actions
        .iter()
        .filter(|(other, _)| **other != action)
        .map(|(other, bound)| (*other, bound.bindings.clone()))
        .collect();
    let Some(bound) = input.actions.get_mut(&action) else {
        return false;
    };
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for (index, binding) in bound.bindings.iter().enumerate() {
            let name = binding.name();
            let shared_with: Vec<&str> = others
                .iter()
                .filter(|(_, bindings)| bindings.contains(binding))
                .map(|(other, _)| <&str>::from(*other))
                .collect();
            let border = if shared_with.is_empty() {
                theme::BORDER
            } else {
                theme::WARN
            };
            chip::holding(ui, theme::RAISED, border, |ui| {
                controls::key_hint(ui, &name);
                if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove").clicked() {
                    remove = Some(index);
                }
            })
            .response
            .on_hover_text(if shared_with.is_empty() {
                format!("Only this action uses {name}.")
            } else {
                format!("Also bound to {}.", shared_with.join(", "))
            });
        }
    });
    let mut changed = false;
    if let Some(index) = remove {
        bound.bindings.remove(index);
        changed = true;
    }
    let frame = input::frame();
    let frame = if capturing {
        frame.stroke(egui::Stroke::new(1.0, theme::ACCENT))
    } else {
        frame
    };
    let response = frame
        .show(ui, |ui| {
            let row = egui::vec2(ui.available_width() - 20.0, input::row_height(ui));
            ui.allocate_ui_with_layout(
                row,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(if capturing {
                        egui::RichText::new("press a key or click…").color(theme::TEXT)
                    } else {
                        theme::muted("+ add a binding")
                    });
                },
            );
        })
        .response
        .interact(Sense::click());
    if response.clicked() && !capturing {
        ui.ctx().data_mut(|d| d.insert_temp(capturing_id, true));
    }
    let kinds: Vec<String> = reads
        .iter()
        .map(|kind| <&str>::from(*kind).replace('_', " "))
        .collect();
    ui.label(theme::faint(format!("The game reads this as: {}", kinds.join(" or "))).size(12.0));
    if !capturing || response.clicked() {
        return changed;
    }
    match ui.input_mut(|input| capture(input, &reads)) {
        Some(Capture::Bound(binding)) => {
            if !bound.bindings.contains(&binding) {
                bound.bindings.push(binding);
                changed = true;
            }
            ui.ctx().data_mut(|d| d.insert_temp(capturing_id, false));
        }
        Some(Capture::Cancelled) => {
            ui.ctx().data_mut(|d| d.insert_temp(capturing_id, false));
        }
        None => {}
    }
    changed
}

enum Capture {
    Bound(InputBinding),
    Cancelled,
}

/// The binding the input's first key or click makes, taken out of the input so the sheet doesn't
/// act on it as well.
fn capture(input: &mut egui::InputState, reads: &[InputBindingKind]) -> Option<Capture> {
    let reads = |kind: InputBindingKind| reads.contains(&kind);
    if input.consume_key(egui::Modifiers::NONE, Key::Escape) {
        return Some(Capture::Cancelled);
    }
    let (index, capture) = input.events.iter().enumerate().find_map(|(index, event)| {
        let binding = match event {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } if reads(InputBindingKind::Key) => key_code(physical_key.unwrap_or(*key))
                .map(|code| InputBinding::key(code).with(held(modifiers))),
            egui::Event::PointerButton {
                button,
                pressed: true,
                modifiers,
                ..
            } => {
                let button = match button {
                    egui::PointerButton::Secondary => MouseButton::Right,
                    egui::PointerButton::Middle => MouseButton::Middle,
                    _ => MouseButton::Left,
                };
                [
                    (InputBindingKind::Click, InputBinding::click(button)),
                    (InputBindingKind::Drag, InputBinding::drag(button)),
                    (InputBindingKind::Mouse, InputBinding::mouse(button)),
                ]
                .into_iter()
                .find_map(|(kind, binding)| reads(kind).then_some(binding))
                .map(|binding| binding.with(held(modifiers)))
            }
            _ => None,
        };
        binding.map(|binding| (index, binding))
    })?;
    input.events.remove(index);
    Some(Capture::Bound(capture))
}

fn held(modifiers: &egui::Modifiers) -> KeyModifiers {
    KeyModifiers {
        ctrl: modifiers.ctrl || modifiers.command,
        shift: modifiers.shift,
        alt: modifiers.alt,
        super_key: false,
    }
}

/// egui and bevy name a few keys apart; content stores bevy's names.
fn key_code(key: Key) -> Option<KeyCode> {
    let name = match key {
        Key::ArrowDown => "ArrowDown".to_owned(),
        Key::ArrowLeft => "ArrowLeft".to_owned(),
        Key::ArrowRight => "ArrowRight".to_owned(),
        Key::ArrowUp => "ArrowUp".to_owned(),
        Key::Equals => "Equal".to_owned(),
        Key::Backtick => "Backquote".to_owned(),
        Key::OpenBracket => "BracketLeft".to_owned(),
        Key::CloseBracket => "BracketRight".to_owned(),
        _ => {
            let name = key.name();
            match name.chars().next() {
                Some(first) if name.len() == 1 && first.is_ascii_alphabetic() => {
                    format!("Key{name}")
                }
                Some(first) if name.len() == 1 && first.is_ascii_digit() => format!("Digit{name}"),
                _ => name.to_owned(),
            }
        }
    };
    KeyCode::deserialize(StrDeserializer::<NameError>::new(&name)).ok()
}
