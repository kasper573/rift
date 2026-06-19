use bevy::prelude::*;
use bevy_view::{View, ViewRoot, view};

use ui::{Button, Text};

use crate::{Screen, auth};

/// The mode chosen before entering play; ordinary accounts always play.
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
            .add_systems(OnEnter(Screen::SigningIn), |c: Commands| {
                screen(c, signing_in)
            })
            .add_systems(OnExit(Screen::SigningIn), despawn)
            .add_systems(OnEnter(Screen::SignInFailed), |c: Commands| {
                screen(c, sign_in_failed)
            })
            .add_systems(OnExit(Screen::SignInFailed), despawn)
            .add_systems(OnEnter(Screen::ChooseMode), |c: Commands| {
                screen(c, choose_mode)
            })
            .add_systems(OnExit(Screen::ChooseMode), despawn)
            .add_systems(OnEnter(Screen::Playing), enter_game);
    }
}

#[derive(Component)]
struct ScreenUi;

fn screen(mut commands: Commands, content: fn(&World) -> View) {
    commands.spawn((
        ScreenUi,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(16.0),
            ..default()
        },
        ViewRoot::new(content),
    ));
}

fn signing_in(_: &World) -> View {
    Text::new("Signing in…").color(Color::WHITE).into()
}

fn sign_in_failed(_: &World) -> View {
    view! {
        { Text::new("Could not sign in").color(Color::WHITE) }
        <Button label="Try again" on:click={|w| set_screen(w, Screen::SigningIn)}/>
    }
}

fn choose_mode(_: &World) -> View {
    view! {
        { Text::new("Choose a mode").color(Color::WHITE) }
        <Button label="Play" on:click={|w| enter(w, Mode::Play)}/>
        <Button label="Spectate" on:click={|w| enter(w, Mode::Spectate)}/>
    }
}

fn enter(world: &mut World, mode: Mode) {
    *world.resource_mut::<Mode>() = mode;
    set_screen(world, Screen::Playing);
}

fn set_screen(world: &mut World, screen: Screen) {
    world.resource_mut::<NextState<Screen>>().set(screen);
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
