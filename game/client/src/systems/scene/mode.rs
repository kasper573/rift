use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Activate, button, text_colored};

use super::Scene;
use crate::core::net;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Play,
    Spectate,
}

pub struct ModePlugin;

impl Plugin for ModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .add_systems(OnEnter(Scene::Mode), choose_mode)
            .add_systems(OnExit(Scene::Mode), crate::systems::despawn_all::<ModeUi>);
    }
}

#[derive(Component, Default, Clone)]
struct ModeUi;

fn choose_mode(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ModeUi
        template_value(super::screen_node())
        Children [
            {EntityScene(text_colored("Choose a mode", Color::WHITE))},
            ( {button("Play")} on(enter(Mode::Play)) ),
            ( {button("Spectate")} on(enter(Mode::Spectate)) ),
        ]
    });
}

fn enter(
    mode: Mode,
) -> impl Fn(On<Activate>, ResMut<Mode>, ResMut<NextState<Scene>>, Commands) + Clone {
    move |_, mut current, mut next, mut commands| {
        *current = mode;
        commands.queue(move |world: &mut World| net::open_session(world, mode == Mode::Spectate));
        next.set(Scene::Connecting);
    }
}
