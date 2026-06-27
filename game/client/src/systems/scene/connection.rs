use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Activate, button, text_colored};

use super::Scene;
use super::mode::Mode;
use crate::core::net;

const OVERLAY_BG: Color = Color::srgb(0.07, 0.07, 0.07);

pub struct ConnectionPlugin;

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Scene::Connecting), connecting)
            .add_systems(OnEnter(Scene::Lost), lost)
            .add_systems(
                OnExit(Scene::Connecting),
                crate::systems::despawn_all::<ConnectionUi>,
            )
            .add_systems(
                OnExit(Scene::Lost),
                crate::systems::despawn_all::<ConnectionUi>,
            );
    }
}

#[derive(Component, Default, Clone)]
struct ConnectionUi;

fn connecting(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ConnectionUi
        template_value(super::screen_node())
        BackgroundColor({OVERLAY_BG})
        GlobalZIndex({100})
        Children [ {EntityScene(text_colored("Connecting...", Color::WHITE))} ]
    });
}

fn lost(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ConnectionUi
        template_value(super::screen_node())
        BackgroundColor({OVERLAY_BG})
        GlobalZIndex({100})
        Children [
            {EntityScene(text_colored("Connection lost", Color::WHITE))},
            ( {button("Reconnect")} on(reconnect) ),
        ]
    });
}

fn reconnect(_: On<Activate>, mode: Res<Mode>, mut commands: Commands) {
    let spectate = *mode == Mode::Spectate;
    commands.queue(move |world: &mut World| net::open_session(world, spectate));
}
