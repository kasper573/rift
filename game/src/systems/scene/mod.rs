pub mod area;
pub mod connection;
pub mod mode;

use bevy::prelude::*;

use crate::core::net::transport::Client;
use crate::systems::player::session;
use connection::{Announce, PendingSession};

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
        .add_systems(Update, (drive, announce));
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
    connection::open_session(world, false);
}

fn announce(world: &mut World) {
    if world.get_resource::<Announce>().is_none() || session::my_id(world).is_none() {
        return;
    }
    let spectate = world.resource::<Announce>().spectate;
    info!(
        "connection welcomed; announcing {}",
        if spectate { "spectate" } else { "join" }
    );
    if spectate {
        session::spectate(world, None);
    } else {
        session::join(world);
    }
    world.remove_resource::<Announce>();
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

pub(crate) fn despawn_all<M: Component>(entities: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}
