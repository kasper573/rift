//! Smoke mode for the end-to-end harness. With `RIFT_CLIENT_SMOKE` set, the real client skips
//! browser sign-in, joins with a bypass session, and exits: success once it has connected,
//! spawned its player, and attached a sprite (so windowing, rendering, and netcode all ran);
//! failure on timeout. This lets the E2E suite drive the actual app, not a stand-in.

use bevy::prelude::*;
use world::protocol::{Actor, Owner};
use world::session::MyClient;

use crate::Screen;

/// How long the client may take to connect, join, spawn, and render before the run fails.
const TIMEOUT: f32 = 30.0;

pub fn enabled() -> bool {
    std::env::var_os("RIFT_CLIENT_SMOKE").is_some()
}

pub struct SmokePlugin;

impl Plugin for SmokePlugin {
    fn build(&self, app: &mut App) {
        if enabled() {
            app.add_systems(Update, succeed.run_if(in_state(Screen::Playing)))
                .add_systems(Update, watchdog);
        }
    }
}

fn succeed(
    me: Res<MyClient>,
    players: Query<&Owner, With<Actor>>,
    rendered: Query<(), (With<Actor>, With<Sprite>)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let spawned = players.iter().any(|owner| owner.client == my);
    if spawned && rendered.iter().next().is_some() {
        info!("smoke: connected, joined, spawned, and rendered a player — success");
        exit.write(AppExit::Success);
    }
}

fn watchdog(time: Res<Time>, mut exit: MessageWriter<AppExit>) {
    if time.elapsed_secs() > TIMEOUT {
        error!("smoke: timed out after {TIMEOUT}s without a spawned, rendered player");
        exit.write(AppExit::error());
    }
}
