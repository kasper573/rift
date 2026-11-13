use bevy::prelude::*;
use bevy_terminal::{
    COMMAND_PREFIX, LineTone, TerminalCompletionRequest, TerminalCompletions, TerminalInput,
};
use ui::egui;
use ui::egui::{Id, Stroke, Ui};
use ui::face::TextRole;
use ui::input::{self, InputField};
use ui::suggest::{self, Suggestion};
use ui::{ink, sheet};

use crate::systems::hud::{self, HudWindow};
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::interface::InterfaceIcon;
use crate::systems::terminal::TerminalLog;
use ui::form;

const TITLE: &str = "Terminal";

pub struct TerminalWindow;

impl HudWindow for TerminalWindow {
    fn title(&self) -> &'static str {
        TITLE
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleTerminal
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Terminal
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let arrived = hud::just_opened(world, TITLE);
        log(ui, world);
        command_line(ui, world, arrived);
    }
}

#[derive(Resource, Default)]
pub(crate) struct TerminalView {
    draft: String,
    /// The draft the server was last asked about, and whether its answer is still out: one
    /// question is out at a time, so typing never asks faster than the server answers.
    asked: Option<String>,
    waiting: bool,
    suggested: Option<TerminalCompletions>,
    selected: usize,
}

impl TerminalView {
    /// What is suggested for the draft as it stands: an answer to an older draft suggests
    /// nothing, as taking one of its options would undo what was typed since.
    fn current(&self) -> Option<&TerminalCompletions> {
        self.suggested
            .as_ref()
            .filter(|suggested| suggested.text == self.draft)
    }
}

pub(super) fn suggested(
    mut answers: MessageReader<TerminalCompletions>,
    mut view: ResMut<TerminalView>,
) {
    for answer in answers.read() {
        view.waiting = false;
        if answer.text == view.draft {
            view.selected = 0;
            view.suggested = Some(answer.clone());
        }
    }
}

fn log(ui: &mut Ui, world: &World) {
    let field = ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 2.0;
    let area = egui::ScrollArea::vertical()
        .id_salt("terminal log")
        .stick_to_bottom(true)
        .auto_shrink(false)
        .max_height((ui.available_height() - field).max(0.0));
    sheet::scroll(ui, area, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for line in world.resource::<TerminalLog>().lines() {
            let color = match line.tone {
                LineTone::Said => ink::INK,
                LineTone::Failed => ink::RUBRIC,
            };
            ui.add(egui::Label::new(TextRole::Mono.rich(line.text.as_str()).color(color)).wrap());
            ui.add_space(2.0);
            form::rule_with(ui, Stroke::new(1.0, ink::ink_wash(0.12)));
            ui.add_space(4.0);
        }
    });
}

// Focus is asked for after the field draws, so the key that opened the terminal is never typed
// into it; a field that sends on its return key loses focus, so it takes it straight back.
fn command_line(ui: &mut Ui, world: &mut World, arrived: bool) {
    let keys = world.resource::<InputMap>();
    let send = keys.shortcut(InputAction::SubmitText);
    let key = |action| keys.shortcut(action);
    let (take, previous, next) = (
        key(InputAction::CompleteText),
        key(InputAction::ChoosePrevious),
        key(InputAction::ChooseNext),
    );
    let mut view = std::mem::take(&mut *world.resource_mut::<TerminalView>());
    let field_id = Id::new("terminal field");
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let options = view
        .current()
        .map_or(0, |suggested| suggested.options.len());
    let mut taken = None;
    if focused && options > 0 {
        let pressed = |shortcut: Option<egui::KeyboardShortcut>| {
            shortcut.is_some_and(|shortcut| ui.input_mut(|input| input.consume_shortcut(&shortcut)))
        };
        if pressed(previous) {
            view.selected = (view.selected + options - 1) % options;
        }
        if pressed(next) {
            view.selected = (view.selected + 1) % options;
        }
        if pressed(take) {
            taken = Some(view.selected);
        }
    }
    let field = ui
        .horizontal(|ui| {
            ui.label(TextRole::Mono.rich(">").color(ink::INK2));
            InputField::new(&mut view.draft)
                .id(field_id)
                .return_key(send)
                .holds_focus()
                .show(ui)
        })
        .inner;
    if let Some(suggested) = view.current().filter(|_| focused) {
        let listed: Vec<Suggestion> = suggested
            .options
            .iter()
            .map(|option| Suggestion {
                text: &option.text,
                detail: &option.detail,
            })
            .collect();
        if !listed.is_empty() || !suggested.usage.is_empty() {
            let clicked = suggest::suggestions(
                ui.ctx(),
                field_id.with("suggestions"),
                field.rect,
                &suggested.usage,
                &listed,
                view.selected,
            );
            taken = taken.or(clicked);
        }
    }
    let completed = taken.and_then(|place| {
        let suggested = view.current()?;
        Some(suggested.applied(suggested.options.get(place)?))
    });
    if let Some(completed) = completed {
        view.draft = completed;
        input::caret_to_end(ui.ctx(), field_id, &view.draft);
        field.request_focus();
    }
    let submitted = field.lost_focus()
        && send.is_some_and(|key| ui.input(|input| input.key_pressed(key.logical_key)));
    if submitted && !view.draft.is_empty() {
        world.write_message(TerminalInput {
            text: std::mem::take(&mut view.draft),
        });
    }
    if arrived || submitted {
        field.request_focus();
    }
    ask(world, &mut view);
    *world.resource_mut::<TerminalView>() = view;
}

/// Asks the server how the draft may go on, once it is a command and no longer what was last
/// asked about.
fn ask(world: &mut World, view: &mut TerminalView) {
    if !view.draft.trim_start().starts_with(COMMAND_PREFIX) {
        view.asked = None;
        view.suggested = None;
        return;
    }
    if view.waiting || view.asked.as_ref() == Some(&view.draft) {
        return;
    }
    view.asked = Some(view.draft.clone());
    view.waiting = true;
    world.write_message(TerminalCompletionRequest {
        text: view.draft.clone(),
    });
}
