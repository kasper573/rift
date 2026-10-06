use bevy::prelude::*;

use super::map::{self, InputAction};
use crate::systems::dialogue::stage;
use crate::systems::history::widget as history;
use crate::systems::{hud, item};

static LAYERS: &[fn(&mut World) -> bool] = &[
    ui::dismiss_topmost,
    item::card::close,
    history::close,
    stage::leave,
    hud::close_topmost_window,
];

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, dismiss_topmost.run_if(not(ui::typing)));
}

fn dismiss_topmost(world: &mut World) {
    if !map::just_pressed(world, InputAction::Dismiss) {
        return;
    }
    for dismiss in LAYERS {
        if dismiss(world) {
            return;
        }
    }
}
