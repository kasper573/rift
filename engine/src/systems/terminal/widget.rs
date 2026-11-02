use bevy::prelude::*;
use bevy_terminal::TerminalInput;
use ui::egui;
use ui::egui::{Id, Stroke, Ui};
use ui::face::TextRole;
use ui::input::InputField;
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
            ui.add(
                egui::Label::new(TextRole::Mono.rich(line.text.as_str()).color(ink::INK)).wrap(),
            );
            ui.add_space(2.0);
            form::rule_with(ui, Stroke::new(1.0, ink::ink_wash(0.12)));
            ui.add_space(4.0);
        }
    });
}

// Focus is asked for after the field draws, so the key that opened the terminal is never typed
// into it; a field that sends on its return key loses focus, so it takes it straight back.
fn command_line(ui: &mut Ui, world: &mut World, arrived: bool) {
    let send = world
        .resource::<InputMap>()
        .shortcut(InputAction::SubmitText);
    let mut draft = std::mem::take(&mut world.resource_mut::<TerminalView>().draft);
    let field = ui
        .horizontal(|ui| {
            ui.label(TextRole::Mono.rich(">").color(ink::INK2));
            InputField::new(&mut draft)
                .id(Id::new("terminal field"))
                .return_key(send)
                .show(ui)
        })
        .inner;
    let submitted = field.lost_focus()
        && send.is_some_and(|key| ui.input(|input| input.key_pressed(key.logical_key)));
    if submitted && !draft.is_empty() {
        world.write_message(TerminalInput {
            text: std::mem::take(&mut draft),
        });
    }
    if arrived || submitted {
        field.request_focus();
    }
    world.resource_mut::<TerminalView>().draft = draft;
}
