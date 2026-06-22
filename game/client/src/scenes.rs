use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::{Activate, button, text_colored};

use crate::{GameScene, net};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Play,
    Spectate,
}

pub struct ScenesPlugin;

impl Plugin for ScenesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .add_systems(OnEnter(GameScene::ChooseMode), choose_mode)
            .add_systems(OnExit(GameScene::ChooseMode), despawn)
            .add_systems(OnEnter(GameScene::Playing), enter_game);
    }
}

#[derive(Component, Default, Clone)]
struct SceneUi;

fn scene_node() -> Node {
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

fn choose_mode(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        SceneUi
        template_value(scene_node())
        Children [
            {EntityScene(text_colored("Choose a mode", Color::WHITE))},
            ( {button("Play")} on(enter(Mode::Play)) ),
            ( {button("Spectate")} on(enter(Mode::Spectate)) ),
        ]
    });
}

fn enter(mode: Mode) -> impl Fn(On<Activate>, ResMut<Mode>, ResMut<NextState<GameScene>>) + Clone {
    move |_, mut current, mut next| {
        *current = mode;
        next.set(GameScene::Playing);
    }
}

fn despawn(ui: Query<Entity, With<SceneUi>>, mut commands: Commands) {
    for entity in &ui {
        commands.entity(entity).despawn();
    }
}

fn enter_game(world: &mut World) {
    let spectate = *world.resource::<Mode>() == Mode::Spectate;
    net::open_session(world, spectate);
}
