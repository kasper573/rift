use bevy::prelude::*;

use crate::systems::dialogue::{history, stage};
use crate::systems::{hud, shop};

static LAYERS: &[fn(&mut World) -> bool] = &[
    ui::dismiss_topmost,
    history::close,
    shop::window::close,
    stage::leave,
    hud::close_topmost_window,
];

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        dismiss_topmost
            .run_if(not(ui::typing))
            .before(ui::UiReactive),
    );
}

fn dismiss_topmost(world: &mut World) {
    if !world
        .resource::<ButtonInput<KeyCode>>()
        .just_pressed(KeyCode::Escape)
    {
        return;
    }
    for dismiss in LAYERS {
        if dismiss(world) {
            return;
        }
    }
}
