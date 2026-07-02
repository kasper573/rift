use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_replicon::prelude::RepliconChannels;
use renet2::{ConnectionConfig, RenetClient};
use ui::{Activate, button, text_colored};

use super::Scene;
use super::mode::Mode;
use crate::core::net::auth::Session;
use crate::core::net::channels::RenetChannelsExt;
use crate::core::net::transport::{Client, Socket};
use crate::core::platform::{StartParams, WsSocket};

const OVERLAY_BG: Color = Color::srgb(0.07, 0.07, 0.07);

pub struct ConnectionPlugin;

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, poll_session)
            .add_systems(OnEnter(Scene::Connecting), connecting)
            .add_systems(OnEnter(Scene::Lost), lost)
            .add_systems(
                OnExit(Scene::Connecting),
                crate::systems::scene::despawn_all::<ConnectionUi>,
            )
            .add_systems(
                OnExit(Scene::Lost),
                crate::systems::scene::despawn_all::<ConnectionUi>,
            );
    }
}

#[derive(Resource)]
pub struct Announce {
    pub spectate: bool,
}

#[derive(Resource)]
pub struct PendingSession {
    task: Task<Result<String, String>>,
    spectate: bool,
}

pub fn open_session(world: &mut World, spectate: bool) {
    let Some(session) = world.get_resource::<Session>().cloned() else {
        error!("no access token; cannot open a session");
        return;
    };
    let game_server_url = world.resource::<StartParams>().game_server_url.clone();
    let task = IoTaskPool::get().spawn_local(fetch_ticket(game_server_url, session.authorization));
    world.insert_resource(PendingSession { task, spectate });
}

fn poll_session(world: &mut World) {
    let Some(mut pending) = world.remove_resource::<PendingSession>() else {
        return;
    };
    match block_on(future::poll_once(&mut pending.task)) {
        Some(Ok(ticket)) => {
            connect(world, ticket.trim());
            world.insert_resource(Announce {
                spectate: pending.spectate,
            });
        }
        Some(Err(error)) => error!("could not open a session: {error}"),
        None => world.insert_resource(pending),
    }
}

async fn fetch_ticket(game_server_url: String, authorization: String) -> Result<String, String> {
    crate::core::platform::fetch(&format!("{game_server_url}/session"), &authorization).await
}

fn connect(world: &mut World, ticket: &str) {
    let channels = world.resource::<RepliconChannels>();
    let connection_config =
        ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs());
    let ws_url = world.resource::<StartParams>().game_server_ws_url.clone();
    // The websocket is reliable+ordered (TCP); renet handles channels and reliability over it.
    let client = RenetClient::new(connection_config, true);
    let socket = WsSocket::open(&format!("{ws_url}/?ticket={ticket}"));
    world.insert_resource(Client(client));
    world.insert_non_send(Socket(socket));
    info!("connection opened");
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
    commands.queue(move |world: &mut World| open_session(world, spectate));
}
