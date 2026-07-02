use bevy::prelude::*;
use bevy_replicon::prelude::{
    ClientMessages, ClientState, ClientStats, ClientSystems, RepliconChannels,
};
use renet2::RenetClient;

use crate::core::platform::ServerSocket;

#[derive(Resource)]
pub struct Client(pub RenetClient);

pub struct Socket(pub Box<dyn ServerSocket>);

pub struct RepliconRenetClientPlugin;

impl Plugin for RepliconRenetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                drive,
                set_connecting.run_if(in_state(ClientState::Disconnected).and_then(connecting)),
                promote.run_if(in_state(ClientState::Connecting).and_then(connecting)),
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

fn drive(socket: Option<NonSend<Socket>>, client: Option<ResMut<Client>>, time: Res<Time>) {
    let (Some(socket), Some(mut client)) = (socket, client) else {
        return;
    };
    client.0.update(time.delta());
    while let Some(packet) = socket.0.recv() {
        client.0.process_packet(&packet);
    }
    // renet has no internal timeout, so a closed socket is the only liveness signal.
    if socket.0.is_closed() {
        client.0.disconnect_due_to_transport();
    }
}

// renet2's core opens in `Connecting` and stays there until told otherwise; with no netcode layer the
// transport promotes it once the socket is up and replicon has reached `Connecting`, so the state
// machine advances Disconnected -> Connecting -> Connected in order.
fn promote(socket: Option<NonSend<Socket>>, client: Option<ResMut<Client>>) {
    let (Some(socket), Some(mut client)) = (socket, client) else {
        return;
    };
    if socket.0.is_open() {
        client.0.set_connected();
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
    socket: NonSend<Socket>,
    mut client: ResMut<Client>,
    mut messages: ResMut<ClientMessages>,
) {
    for (channel_id, message) in messages.drain_sent() {
        client.0.send_message(channel_id as u8, message);
    }
    for packet in client.0.get_packets_to_send() {
        socket.0.send(&packet);
    }
}

fn connecting(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connecting())
}

fn connected(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connected())
}

fn connection_lost(state: Res<State<ClientState>>, client: Option<Res<Client>>) -> bool {
    !matches!(state.get(), ClientState::Disconnected)
        && client.is_some_and(|client| client.0.is_disconnected())
}
