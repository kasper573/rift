use bevy::prelude::*;
use design_tokens::{color, widgets};
use egui::{Id, LayerId, Order, RichText};

use super::{ResetAnswerRequest, ResetPrompt};
use crate::core::gui;
use crate::systems::input::map::{InputAction, InputMap};
use crate::systems::player::session;

const PROMPT: &str = "reset prompt";

/// Whether the question on the character has been answered, until the server takes it away.
#[derive(Resource, Default)]
pub struct PromptState {
    answered: bool,
    focused: bool,
}

pub fn showing(world: &World) -> bool {
    prompt(world).is_some() && !world.resource::<PromptState>().answered
}

pub fn layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new(PROMPT))
}

/// Answers Keep, as closing the prompt any other way does.
pub fn keep(world: &mut World) -> bool {
    if !showing(world) {
        return false;
    }
    answer(world, false);
    true
}

pub fn forget(mut state: ResMut<PromptState>) {
    *state = PromptState::default();
}

pub fn draw_prompt(world: &mut World) {
    let Some(asked) = prompt(world) else {
        let mut state = world.resource_mut::<PromptState>();
        state.answered = false;
        state.focused = false;
        return;
    };
    if world.resource::<PromptState>().answered {
        return;
    }
    let Some(gui) = gui::context(world) else {
        return;
    };
    let keep_key = world
        .resource::<InputMap>()
        .shortcut(InputAction::TakeDefault);
    let focused = world.resource::<PromptState>().focused;
    let mut decided = None;
    let modal = egui::Modal::new(Id::new(PROMPT)).show(&gui, |ui| {
        ui.set_max_width(360.0);
        ui.heading("Reset your skills?");
        ui.add_space(4.0);
        ui.label(asked.describe());
        ui.label(
            RichText::new(
                "Hotbar slots that held them stay, dimmed, until they are learned again.",
            )
            .color(color::MUTED),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let keep = ui.button("Keep");
                if !focused {
                    keep.request_focus();
                }
                if widgets::danger(ui, "Reset").clicked() {
                    decided = Some(true);
                }
                let keep_pressed =
                    keep_key.is_some_and(|key| ui.input_mut(|i| i.consume_shortcut(&key)));
                if keep.clicked() || keep_pressed {
                    decided = Some(false);
                }
            });
        });
    });
    world.resource_mut::<PromptState>().focused = true;
    if modal.should_close() && decided.is_none() {
        decided = Some(false);
    }
    if let Some(accepted) = decided {
        answer(world, accepted);
    }
}

fn prompt(world: &World) -> Option<ResetPrompt> {
    session::my_character(world).and_then(|me| me.get::<ResetPrompt>().copied())
}

fn answer(world: &mut World, accepted: bool) {
    world.resource_mut::<PromptState>().answered = true;
    world.write_message(ResetAnswerRequest { accepted });
}
