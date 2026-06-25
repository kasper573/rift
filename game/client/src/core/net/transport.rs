//! The Replicon ↔ renet2 client backend, hand-rolled.
//!
//! This is the in-repo stand-in for `bevy_replicon_renet2`'s `RepliconRenetClientPlugin` plus
//! `bevy_renet2`'s client/netcode driving systems — it pumps messages between [`RenetClient`] and
//! Replicon's [`ClientMessages`] and tracks [`ClientState`]. We carry it only because those crates
//! are still pinned to bevy 0.18; once they publish a bevy-0.19 build this whole module can be
//! deleted and `NetPlugin` can add `bevy_replicon_renet2::RepliconRenetPlugins` instead, inserting
//! `RenetClient`/`NetcodeClientTransport` directly (they become `Resource`s under that crate's
//! `bevy` feature, which is why we wrap them here). See [`world::core::channels`] for the matching channel seam.

use bevy::prelude::*;
use bevy_replicon::prelude::{
    ClientMessages, ClientState, ClientStats, ClientSystems, RepliconChannels,
};
use renet2::RenetClient;
use renet2_netcode::NetcodeClientTransport;

/// Wraps renet2's client so Bevy can hold it as a resource (renet2's own `bevy` feature, which would
/// derive `Resource`, is off because it pulls an incompatible bevy_ecs).
#[derive(Resource)]
pub struct Client(pub RenetClient);

/// Wraps the netcode transport as a resource, for the same reason as [`Client`].
#[derive(Resource)]
pub struct Transport(pub NetcodeClientTransport);

pub struct RepliconRenetClientPlugin;

impl Plugin for RepliconRenetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                drive,
                set_connecting.run_if(in_state(ClientState::Disconnected).and_then(connecting)),
                set_connected.run_if(in_state(ClientState::Connecting).and_then(connected)),
                set_disconnected.run_if(connection_lost),
                receive_packets.run_if(connected),
            )
                .chain()
                .in_set(ClientSystems::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .run_if(connected)
                .in_set(ClientSystems::SendPackets),
        );
    }
}

/// Advances the client and lets the transport deliver inbound packets into it. Driven by virtual time,
/// not `Time<Real>`: its delta is clamped (`max_delta`), so a long first-render frame (pipeline
/// builds and uploads, slow on software GPUs) can't be charged to netcode's liveness timer as a network
/// gap and trip the connection timeout — the stall is the client being busy, not the link being dead.
fn drive(client: Option<ResMut<Client>>, transport: Option<ResMut<Transport>>, time: Res<Time>) {
    let Some(mut client) = client else {
        return;
    };
    client.0.update(time.delta());
    if let Some(mut transport) = transport
        && let Err(error) = transport.0.update(time.delta(), &mut client.0)
    {
        error!("netcode transport update failed: {error}");
    }
}

fn set_connecting(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Connecting);
}

fn set_connected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Connected);
}

fn set_disconnected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Disconnected);
}

fn receive_packets(
    channels: Res<RepliconChannels>,
    mut client: ResMut<Client>,
    mut messages: ResMut<ClientMessages>,
    mut stats: ResMut<ClientStats>,
) {
    for channel_id in 0..channels.server_channels().len() as u8 {
        while let Some(message) = client.0.receive_message(channel_id) {
            messages.insert_received(channel_id, message);
        }
    }
    stats.rtt = client.0.rtt();
    stats.packet_loss = client.0.packet_loss();
    stats.sent_bps = client.0.bytes_sent_per_sec();
    stats.received_bps = client.0.bytes_received_per_sec();
}

fn send_packets(
    mut client: ResMut<Client>,
    mut transport: ResMut<Transport>,
    mut messages: ResMut<ClientMessages>,
) {
    for (channel_id, message) in messages.drain_sent() {
        client.0.send_message(channel_id as u8, message);
    }
    if let Err(error) = transport.0.send_packets(&mut client.0) {
        error!("netcode transport send failed: {error}");
    }
}

fn connecting(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connecting())
}

fn connected(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connected())
}

/// True the moment a live link drops — whether an established connection was lost or the initial
/// handshake timed out without ever connecting. Idempotent once [`ClientState::Disconnected`] is set.
fn connection_lost(state: Res<State<ClientState>>, client: Option<Res<Client>>) -> bool {
    !matches!(state.get(), ClientState::Disconnected)
        && client.is_some_and(|client| client.0.is_disconnected())
}
