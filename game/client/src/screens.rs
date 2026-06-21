use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Activate, button, text_colored};

use crate::{Screen, auth};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Play,
    Spectate,
}

pub struct ScreensPlugin;

impl Plugin for ScreensPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .add_systems(OnEnter(Screen::SigningIn), signing_in)
            .add_systems(OnExit(Screen::SigningIn), despawn)
            .add_systems(OnEnter(Screen::SignInFailed), sign_in_failed)
            .add_systems(OnExit(Screen::SignInFailed), despawn)
            .add_systems(OnEnter(Screen::ChooseMode), choose_mode)
            .add_systems(OnExit(Screen::ChooseMode), despawn)
            .add_systems(OnEnter(Screen::Playing), enter_game);
    }
}

#[derive(Component, Default, Clone)]
struct ScreenUi;

fn screen_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(16.0),
        ..default()
    }
}

fn signing_in(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ScreenUi
        template_value(screen_node())
        Children [ {EntityScene(text_colored("Signing in…", Color::WHITE))} ]
    });
}

fn sign_in_failed(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ScreenUi
        template_value(screen_node())
        Children [
            {EntityScene(text_colored("Could not sign in", Color::WHITE))},
            (
                {button("Try again")}
                on(|_: On<Activate>, mut next: ResMut<NextState<Screen>>| {
                    next.set(Screen::SigningIn);
                })
            ),
        ]
    });
}

fn choose_mode(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        ScreenUi
        template_value(screen_node())
        Children [
            {EntityScene(text_colored("Choose a mode", Color::WHITE))},
            ( {button("Play")} on(enter(Mode::Play)) ),
            ( {button("Spectate")} on(enter(Mode::Spectate)) ),
        ]
    });
}

fn enter(mode: Mode) -> impl Fn(On<Activate>, ResMut<Mode>, ResMut<NextState<Screen>>) + Clone {
    move |_, mut current, mut next| {
        *current = mode;
        next.set(Screen::Playing);
    }
}

fn despawn(ui: Query<Entity, With<ScreenUi>>, mut commands: Commands) {
    for entity in &ui {
        commands.entity(entity).despawn();
    }
}

fn enter_game(world: &mut World) {
    let spectate = *world.resource::<Mode>() == Mode::Spectate;
    auth::enter(world, spectate);
}
