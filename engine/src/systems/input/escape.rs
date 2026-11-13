use bevy::prelude::*;
use ui::egui::LayerId;

use super::map::{self, InputAction, InputMap};
use super::scope::InputScope;
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::{crafting, hud, item, resource, skill, terminal};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, dismiss_topmost.run_if(resource_exists::<InputMap>));
}

fn dismiss_topmost(world: &mut World) {
    if !map::just_pressed(world, InputAction::Dismiss) {
        return;
    }
    // A field in a window and an open list give the keyboard back themselves on the press.
    if *world.resource::<InputScope>() == InputScope::Typing {
        terminal::chat::hand_back(world);
        return;
    }
    if skill::bar::cancel_rebind(world) || skill::aim::disarm(world) {
        return;
    }
    let Some(gui) = gui::context(world) else {
        return;
    };
    let layers: Vec<LayerId> = gui.memory(|memory| memory.layer_ids().collect());
    let open = dismissals(world);
    let topmost = layers.iter().rev().find_map(|layer| {
        open.iter()
            .find(|(open_layer, _)| open_layer == layer)
            .map(|(_, dismissal)| dismissal)
    });
    match topmost {
        Some(dismissal) => {
            dismissal.run(world);
        }
        None => resource::render::stop(world),
    }
}

enum Dismissal {
    Confirm,
    Window(&'static str),
    ItemCard,
    SkillCard,
    Conversation,
}

impl Dismissal {
    fn run(&self, world: &mut World) -> bool {
        match self {
            Dismissal::Confirm => hud::dismiss_confirm(world),
            Dismissal::Window(title) => hud::close_window(world, title),
            Dismissal::ItemCard => item::card::close(world),
            Dismissal::SkillCard => skill::card::close(world),
            Dismissal::Conversation => crafting::panel::cancel(world) || stage::leave(world),
        }
    }
}

fn dismissals(world: &World) -> Vec<(LayerId, Dismissal)> {
    let mut open: Vec<(LayerId, Dismissal)> = hud::open_windows(world)
        .map(|title| (hud::window_layer(title), Dismissal::Window(title)))
        .collect();
    if hud::confirming(world) {
        open.push((hud::confirm_layer(), Dismissal::Confirm));
    }
    if item::card::showing(world) {
        open.push((item::card::layer(), Dismissal::ItemCard));
    }
    if skill::card::showing(world) {
        open.push((skill::card::layer(), Dismissal::SkillCard));
    }
    if stage::leavable(world) {
        open.push((stage::layer(), Dismissal::Conversation));
    }
    open
}
