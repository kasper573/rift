use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy_replicon_renet2::RepliconRenetClientPlugin;
use bevy_replicon_renet2::renet2::{RenetClient, RenetClientPlugin, RenetReceive, RenetSend};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

pub struct ClientSocket(pub ServerSocket);

/// Carries renet2 over a socket the platform opened, with no netcode layer: the socket itself is
/// what connects and disconnects the client.
pub struct SocketClientPlugin;

impl Plugin for SocketClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RepliconRenetClientPlugin)
            .add_systems(
                PreUpdate,
                receive
                    .in_set(RenetReceive)
                    .after(RenetClientPlugin::update_system),
            )
            .add_systems(PostUpdate, send.in_set(RenetSend));
    }
}

fn receive(
    mut commands: Commands,
    socket: Option<NonSend<ClientSocket>>,
    client: Option<ResMut<RenetClient>>,
) {
    let (Some(socket), Some(mut client)) = (socket, client) else {
        return;
    };
    while let Some(packet) = socket.0.recv() {
        client.process_packet(&packet);
    }
    match socket.0.state() {
        SocketState::Open if client.is_connecting() => client.set_connected(),
        // renet has no internal timeout, so a closed socket is the only liveness signal. The
        // connection ends with its resources: replicon starts a connection when a client resource is
        // added, so the next one must not overwrite this one.
        SocketState::Closed => {
            commands.remove_resource::<RenetClient>();
            commands.queue(|world: &mut World| {
                world.remove_non_send::<ClientSocket>();
            });
        }
        SocketState::Connecting | SocketState::Open => {}
    }
}

fn send(socket: Option<NonSend<ClientSocket>>, client: Option<ResMut<RenetClient>>) {
    let (Some(socket), Some(mut client)) = (socket, client) else {
        return;
    };
    for packet in client.get_packets_to_send() {
        socket.0.send(packet);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketState {
    Connecting,
    Open,
    Closed,
}

/// Carries opaque renet packets over a WebSocket on a thread of its own, with no netcode layer, so
/// frames are buffered and drained each tick.
pub struct ServerSocket {
    incoming: Mutex<Receiver<Vec<u8>>>,
    outgoing: Sender<Vec<u8>>,
    state: Arc<Mutex<SocketState>>,
}

impl ServerSocket {
    pub fn open(url: &str) -> ServerSocket {
        let (incoming_tx, incoming) = channel();
        let (outgoing, outgoing_rx) = channel();
        let state = Arc::new(Mutex::new(SocketState::Connecting));
        std::thread::spawn({
            let state = state.clone();
            let url = url.to_owned();
            move || drive(&url, &state, &incoming_tx, &outgoing_rx)
        });
        ServerSocket {
            incoming: Mutex::new(incoming),
            outgoing,
            state,
        }
    }

    pub fn recv(&self) -> Option<Vec<u8>> {
        self.incoming.lock().expect("socket lock").try_recv().ok()
    }

    pub fn send(&self, packet: &[u8]) {
        if self.state() == SocketState::Open {
            let _ = self.outgoing.send(packet.to_vec());
        }
    }

    pub fn state(&self) -> SocketState {
        *self.state.lock().expect("socket state lock")
    }
}

fn drive(
    url: &str,
    state: &Mutex<SocketState>,
    incoming: &Sender<Vec<u8>>,
    outgoing: &Receiver<Vec<u8>>,
) {
    let set = |to: SocketState| *state.lock().expect("socket state lock") = to;
    let Ok((mut ws, _)) = tungstenite::connect(url).map_err(|error| eprintln!("{url}: {error}"))
    else {
        return set(SocketState::Closed);
    };
    let polling = match ws.get_ref() {
        MaybeTlsStream::Plain(tcp) => tcp.set_read_timeout(Some(READ_POLL)),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(Some(READ_POLL)),
        _ => Ok(()),
    };
    if polling.is_err() {
        return set(SocketState::Closed);
    }
    set(SocketState::Open);
    let closed = || set(SocketState::Closed);
    loop {
        loop {
            match outgoing.try_recv() {
                Ok(packet) => match ws.send(Message::Binary(packet.into())) {
                    Ok(()) => {}
                    Err(tungstenite::Error::Io(error)) if retryable(&error) => {}
                    Err(_) => return closed(),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return closed(),
            }
        }
        let _ = ws.flush();
        match ws.read() {
            Ok(Message::Binary(data)) => {
                if incoming.send(data.to_vec()).is_err() {
                    return closed();
                }
            }
            Ok(Message::Close(_)) => return closed(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if retryable(&error) => {}
            Err(_) => return closed(),
        }
    }
}

// A socket call with a timeout fails as interrupted when the process is stopped and resumed (a
// debugger, a suspended terminal job), so that is no more the connection's end than a poll that
// found nothing.
fn retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

const READ_POLL: Duration = Duration::from_millis(1);
