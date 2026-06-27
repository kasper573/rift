pub mod area;
pub mod connection;
pub mod mode;

use bevy::prelude::*;

use crate::core::net::transport::Client;
use crate::core::net::{self, PendingSession};

#[derive(States, Default, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Scene {
    #[default]
    Mode,
    Connecting,
    Lost,
    Area,
}

pub struct ScenePlugin {
    pub spectator: bool,
}

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(if self.spectator {
            Scene::Mode
        } else {
            Scene::Connecting
        })
        .add_plugins((
            mode::ModePlugin,
            connection::ConnectionPlugin,
            area::AreaPlugin,
        ))
        .add_systems(Update, drive);
        if !self.spectator {
            app.add_systems(Startup, begin);
        }
    }
}

fn drive(
    pending: Option<Res<PendingSession>>,
    client: Option<Res<Client>>,
    scene: Res<State<Scene>>,
    mut next: ResMut<NextState<Scene>>,
) {
    if *scene.get() == Scene::Mode {
        return;
    }
    let connected = client
        .as_ref()
        .is_some_and(|client| client.0.is_connected());
    let attempting = pending.is_some()
        || client
            .as_ref()
            .is_some_and(|client| client.0.is_connecting());
    let target = if connected {
        Scene::Area
    } else if attempting {
        Scene::Connecting
    } else {
        Scene::Lost
    };
    if *scene.get() != target {
        next.set(target);
    }
}

fn begin(world: &mut World) {
    net::open_session(world, false);
}

fn screen_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(16.0),
        ..default()
    }
}
