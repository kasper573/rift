use std::collections::HashMap;
use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use renet2::RenetServer;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use world::systems::account::Identity;

use crate::Sessions;

/// Drives renet2's transport-agnostic core directly over plain WebSockets, with no netcode layer.
/// There is therefore no fixed connection cap (renetcode's was 1024); the only bound is `max_clients`,
/// a soft resource limit. TLS is terminated by the reverse proxy and the user is already authenticated
/// by the ticket, so netcode's encryption and anti-spoof challenge would add nothing here.
///
/// Each renet packet is one binary WebSocket frame; renet still owns channels and reliability. The
/// accept loop and per-connection reader/writer tasks run on the shared tokio runtime and reach the
/// single-threaded game loop only through these channels, so the loop never blocks on IO.
pub struct WsTransport {
    events: mpsc::UnboundedReceiver<Event>,
    senders: HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>,
    identities: HashMap<u64, Identity>,
    max_clients: usize,
}

enum Event {
    Connected {
        client: u64,
        identity: Identity,
        out: mpsc::UnboundedSender<Vec<u8>>,
    },
    Packet {
        client: u64,
        data: Vec<u8>,
    },
    Disconnected {
        client: u64,
    },
}

impl WsTransport {
    pub fn bind(
        addr: SocketAddr,
        sessions: Sessions,
        max_clients: usize,
        runtime: Handle,
    ) -> WsTransport {
        let (events_tx, events) = mpsc::unbounded_channel();
        runtime.spawn(accept(addr, sessions, events_tx));
        WsTransport {
            events,
            senders: HashMap::new(),
            identities: HashMap::new(),
            max_clients,
        }
    }

    /// Applies every connect, packet, and disconnect that arrived since the last tick to the renet
    /// server. renet has no internal timeout, so a closed socket (the `Disconnected` event) is the
    /// sole liveness signal — it must reach `remove_connection` or the connection lingers forever.
    pub fn update(&mut self, server: &mut RenetServer) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected {
                    client,
                    identity,
                    out,
                } => {
                    if self.senders.len() >= self.max_clients {
                        continue; // server full: dropping `out` closes the writer task's socket
                    }
                    self.senders.insert(client, out);
                    self.identities.insert(client, identity);
                    server.add_connection(client, true);
                }
                Event::Packet { client, data } => {
                    let _ = server.process_packet_from(&data, client);
                }
                Event::Disconnected { client } => {
                    self.senders.remove(&client);
                    self.identities.remove(&client);
                    server.remove_connection(client);
                }
            }
        }
    }

    /// The identity the ticket authenticated, handed to the game loop when it processes the matching
    /// renet `ClientConnected` event.
    pub fn take_identity(&mut self, client: u64) -> Option<Identity> {
        self.identities.remove(&client)
    }

    pub fn send(&mut self, server: &mut RenetServer) {
        for client in server.clients_id() {
            let Some(out) = self.senders.get(&client) else {
                continue;
            };
            if let Ok(packets) = server.get_packets_to_send(client) {
                for packet in packets {
                    if out.send(packet).is_err() {
                        break; // writer task gone; the reader will report the disconnect
                    }
                }
            }
        }
    }
}

async fn accept(addr: SocketAddr, sessions: Sessions, events: mpsc::UnboundedSender<Event>) {
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("cannot bind websocket {addr}: {error}"));
    println!("websocket transport listening on {addr}");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        tokio::spawn(serve(stream, sessions.clone(), events.clone()));
    }
}

// The handshake callback's `Result<Response, ErrorResponse>` is tungstenite's signature; its `Err`
// variant is a whole HTTP response, so it cannot be shrunk here.
#[allow(clippy::result_large_err)]
async fn serve(stream: TcpStream, sessions: Sessions, events: mpsc::UnboundedSender<Event>) {
    // Claim the single-use ticket inside the handshake callback, so two sockets bearing the same
    // ticket can never both become a connection (the second `take` returns `None` and is rejected).
    let mut claimed: Option<(u64, Identity)> = None;
    let handshake = accept_hdr_async(
        stream,
        |request: &Request, response: Response| match ticket(request)
            .and_then(|id| sessions.take(id).map(|identity| (id, identity)))
        {
            Some(pair) => {
                claimed = Some(pair);
                Ok(response)
            }
            None => Err(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Some("invalid or expired ticket".to_owned()))
                .expect("reject response")),
        },
    )
    .await;
    let (Ok(ws), Some((client, identity))) = (handshake, claimed) else {
        return;
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    if events
        .send(Event::Connected {
            client,
            identity,
            out: out_tx,
        })
        .is_err()
    {
        return;
    }

    let (mut sink, mut incoming) = ws.split();
    tokio::spawn(async move {
        while let Some(data) = out_rx.recv().await {
            if sink.send(Message::Binary(data.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(message) = incoming.next().await {
        match message {
            Ok(Message::Binary(data)) => {
                if events
                    .send(Event::Packet {
                        client,
                        data: data.to_vec(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let _ = events.send(Event::Disconnected { client });
}

fn ticket(request: &Request) -> Option<u64> {
    request
        .uri()
        .query()?
        .split('&')
        .find_map(|pair| pair.strip_prefix("ticket="))
        .and_then(|value| value.parse().ok())
}
