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
        // A field in a window and an open list give the keyboard back themselves on the press.
        InputScope::Typing => terminal::chat::hand_back(world),
        InputScope::Modal => hud::dismiss_confirm(world),
        InputScope::Conversation => {
            flick_card(world) || crafting::panel::cancel(world) || stage::leave(world)
        }
        InputScope::Menu => stop_using_the_bar(world) || flick_card(world) || menu::close(world),
        InputScope::World => {
            stop_using_the_bar(world) || flick_card(world) || resource::render::stop(world)
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

fn flick_card(world: &mut World) -> bool {
    let item_in_front = gui::context(world).is_some_and(|gui| {
        gui.memory(|memory| {
            let place = |layer| memory.layer_ids().position(|drawn| drawn == layer);
            place(item::card::layer()) > place(skill::card::layer())
        })
    });
    let front_first: [fn(&mut World) -> bool; 2] = if item_in_front {
        [item::card::close, skill::card::close]
    } else {
        [skill::card::close, item::card::close]
    };
    front_first.into_iter().any(|close| close(world))
}
