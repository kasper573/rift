use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use egui::LayerId;

use super::map::{self, InputAction, InputMap};
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::{hud, item, skill};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        dismiss_topmost
            .run_if(not(egui_wants_any_keyboard_input))
            .run_if(resource_exists::<InputMap>),
    );
}

fn dismiss_topmost(world: &mut World) {
    if !map::just_pressed(world, InputAction::Dismiss) {
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
    if let Some(dismissal) = topmost {
        dismissal.run(world);
    }
}

enum Dismissal {
    Confirm,
    Window(&'static str),
    ItemCard,
    SkillCard,
    ResetPrompt,
    Conversation,
}

impl Dismissal {
    fn run(&self, world: &mut World) -> bool {
        match self {
            Dismissal::Confirm => hud::dismiss_confirm(world),
            Dismissal::Window(title) => hud::close_window(world, title),
            Dismissal::ItemCard => item::card::close(world),
            Dismissal::SkillCard => skill::card::close(world),
            Dismissal::ResetPrompt => skill::prompt::keep(world),
            Dismissal::Conversation => stage::leave(world),
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
    if skill::prompt::showing(world) {
        open.push((skill::prompt::layer(), Dismissal::ResetPrompt));
    }
    if stage::leavable(world) {
        open.push((stage::layer(), Dismissal::Conversation));
    }
    open
}
