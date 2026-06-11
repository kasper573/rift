use bevy::prelude::*;

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
                notice(c, "Signing in…")
            })
            .add_systems(OnExit(Screen::SigningIn), despawn)
            .add_systems(OnEnter(Screen::SignInFailed), sign_in_failed)
            .add_systems(OnExit(Screen::SignInFailed), despawn)
            .add_systems(OnEnter(Screen::ChooseMode), choose_mode)
            .add_systems(OnExit(Screen::ChooseMode), despawn)
            .add_systems(OnEnter(Screen::Playing), enter_game);
    }
}

#[derive(Component)]
struct ScreenUi;

fn screen() -> Node {
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

fn label(text: &str) -> impl Bundle {
    (Text::new(text), TextColor(Color::WHITE))
}

fn button(text: &str) -> impl Bundle {
    (
        Button,
        Node {
            padding: UiRect::axes(Val::Px(20.0), Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(Color::WHITE),
        BackgroundColor(Color::BLACK),
        children![label(text)],
    )
}

fn notice(mut commands: Commands, text: &str) {
    commands.spawn((ScreenUi, screen(), children![label(text)]));
}

fn sign_in_failed(mut commands: Commands) {
    commands
        .spawn((ScreenUi, screen(), children![label("Could not sign in")]))
        .with_child(button("Try again"))
        .observe(
            |_: On<Pointer<Click>>, mut screen: ResMut<NextState<Screen>>| {
                screen.set(Screen::SigningIn);
            },
        );
}

 fn choose_mode(mut commands: Commands) {
      commands
          .spawn((ScreenUi, screen(), children![label("Choose a mode")]))
          .with_children(|screen_ui| {
              screen_ui.spawn(button("Play")).observe(
                  |_: On<Pointer<Click>>, mut mode: ResMut<Mode>, mut screen: ResMut<NextState<Screen>>| {
                      *mode = Mode::Play;
                      screen.set(Screen::Playing);
                  },
              );
              screen_ui.spawn(button("Spectate")).observe(
                  |_: On<Pointer<Click>>, mut mode: ResMut<Mode>, mut screen: ResMut<NextState<Screen>>| {
                      *mode = Mode::Spectate;
                      screen.set(Screen::Playing);
                  },
              );
          });
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
