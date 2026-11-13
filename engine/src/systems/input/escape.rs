use bevy::prelude::*;

use super::map::{self, InputAction, InputMap};
use super::scope::InputScope;
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::hud::menu;
use crate::systems::{crafting, hud, item, resource, skill, terminal};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, step_out.run_if(resource_exists::<InputMap>));
}

fn step_out(world: &mut World) {
    if !map::just_pressed(world, InputAction::Dismiss) {
        return;
    }
    let scope = *world.resource::<InputScope>();
    let stepped = match scope {
        InputScope::Typing => {
            let chat = terminal::chat::hand_back(world);
            gui::release_keyboard(world) || chat
        }
        InputScope::Modal => hud::dismiss_confirm(world),
        InputScope::Conversation => {
            close_front(world) || crafting::panel::cancel(world) || stage::leave(world)
        }
        InputScope::Menu => stop_using_the_bar(world) || close_front(world),
        InputScope::World => {
            stop_using_the_bar(world) || close_front(world) || resource::render::stop(world)
        }
    };
    if !stepped && scope == InputScope::World {
        menu::open(world);
    }
}

// The bar's keys are read under the menu as in the world, so what they began is stepped out of
// in both.
fn stop_using_the_bar(world: &mut World) -> bool {
    skill::bar::cancel_rebind(world) || skill::aim::disarm(world)
}

/// The menu and the cards stand in a stack, and a press steps out of the one in front: whichever
/// the player opened or reached for last. One that is already on its way out is passed over.
fn close_front(world: &mut World) -> bool {
    hud::sheets_front_first(world)
        .into_iter()
        .any(|sheet| match sheet {
            menu::WINDOW_KEY => menu::close(world),
            item::card::WINDOW_KEY => item::card::close(world),
            skill::card::WINDOW_KEY => skill::card::close(world),
            _ => false,
        })
}
