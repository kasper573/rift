use std::collections::HashMap;

use bevy::prelude::*;
use bevy_terminal::TerminalInput;
use design_tokens::widgets;
use egui::{Id, Ui};

use crate::systems::hud::{self, HudWindow};
use crate::systems::input::map::{self, InputAction};
use crate::systems::interface::InterfaceIcon;
use crate::systems::terminal::{TerminalKind, Terminals};

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
        let tabs = world.resource::<Terminals>().tabs.clone();
        let Some(&first) = tabs.first() else {
            ui.label("Waiting for server…");
            return;
        };
        let shown = world
            .resource::<TerminalView>()
            .tab
            .filter(|tab| tabs.contains(tab));
        let mut tab = shown.unwrap_or(first);
        let titles: Vec<(TerminalKind, String)> = tabs
            .iter()
            .map(|&kind| (kind, <&str>::from(kind).to_owned()))
            .collect();
        widgets::tab_strip(ui, &titles, &mut tab);
        let arrived = shown != Some(tab) || hud::just_opened(world, TITLE);
        world.resource_mut::<TerminalView>().tab = Some(tab);
        log(ui, world, tab);
        command_line(ui, world, tab, arrived);
    }
}

#[derive(Resource, Default)]
pub(crate) struct TerminalView {
    tab: Option<TerminalKind>,
    drafts: HashMap<TerminalKind, String>,
}

fn log(ui: &mut Ui, world: &World, tab: TerminalKind) {
    let field = ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 2.0;
    egui::ScrollArea::vertical()
        .id_salt(("terminal log", <&str>::from(tab)))
        .stick_to_bottom(true)
        .auto_shrink(false)
        .max_height((ui.available_height() - field).max(0.0))
        .show(ui, |ui| {
            for (_, line) in world.resource::<Terminals>().lines(tab) {
                ui.label(line.as_str());
            }
        });
}

// Focus is asked for after the field draws, so the key that opened the terminal is never typed
// into it; a field that sends on its return key loses focus, so it takes it straight back.
fn command_line(ui: &mut Ui, world: &mut World, tab: TerminalKind, arrived: bool) {
    let send = map::shortcut(world, InputAction::SubmitText);
    let mut draft = world
        .resource_mut::<TerminalView>()
        .drafts
        .remove(&tab)
        .unwrap_or_default();
    let field = ui.add(
        egui::TextEdit::singleline(&mut draft)
            .id(Id::new(("terminal field", <&str>::from(tab))))
            .desired_width(f32::INFINITY)
            .return_key(send),
    );
    let submitted = field.lost_focus()
        && send.is_some_and(|key| ui.input(|input| input.key_pressed(key.logical_key)));
    if submitted && !draft.is_empty() {
        world.write_message(TerminalInput {
            terminal: tab,
            text: std::mem::take(&mut draft),
        });
    }
    if arrived || submitted {
        field.request_focus();
    }
    world
        .resource_mut::<TerminalView>()
        .drafts
        .insert(tab, draft);
}
