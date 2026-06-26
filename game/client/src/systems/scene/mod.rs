//! The client's top-level scenes — the single screen the player sees at any moment, collected here so
//! the whole flow reads in one place. Exactly one [`Scene`] is active: the [`mode`] picker, the
//! [`connection`] overlay (while the link is [`Connecting`](Scene::Connecting) or [`Lost`](Scene::Lost)),
//! or the live [`area`]. [`Scene`] is the single source of truth for which screen is up, [`drive`]n
//! from the netcode connection.

pub mod area;
pub mod connection;
pub mod mode;

use bevy::prelude::*;

use crate::core::net::transport::Client;
use crate::core::net::{self, PendingSession};

#[derive(States, Default, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Scene {
    /// Spectators choose to play or watch here; players skip straight to connecting.
    #[default]
    Mode,
    /// Opening (or re-opening) the game-server session.
    Connecting,
    /// The link dropped or never came up; offers a reconnect.
    Lost,
    /// Connected and in the world — the game itself.
    Area,
}

/// Wires the scene state machine: the per-scene plugins, the [`drive`] that reflects the connection
/// into the [`Scene`], and the player's immediate connect (spectators wait for the [`mode`] pick).
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

/// Reflects the live connection into the [`Scene`]: once past the [`Mode`](Scene::Mode) picker the
/// scene is the [`Area`](Scene::Area) when connected, [`Connecting`](Scene::Connecting) while an attempt
/// is in flight, else [`Lost`](Scene::Lost). Leaving [`Scene::Mode`] is the picker's job, so the driver
/// leaves it untouched. Reads the client directly rather than replicon's frame-late `ClientState`.
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

/// A player connects the instant the app boots — there is no mode to choose.
fn begin(world: &mut World) {
    net::open_session(world, false);
}

/// The shared root layout for a scene's full-screen content: a centered column covering the viewport.
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
