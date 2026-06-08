use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Instant;

use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::{Message, Role, WebSocket, WebSocketConfig};

use super::http;
use crate::app::Feature;
use crate::cluster::Cluster;
use crate::link::{MAX_MESSAGE_BYTES, frame, pump, unframe_one};
use crate::metrics::Metrics;
use crate::server::Session;
use crate::world::ClientId;

/// Validates a handshake token into the connection's [`Session`]; `Err` refuses the connection.
/// Without one, every connection is admitted.
pub type Authenticator = Box<dyn FnMut(&str) -> Result<Session, String> + Send>;

/// A TCP front-end for a sharded [`Cluster`]. One port, three protocols sniffed from the first
/// bytes: raw rift framing (the first frame is the handshake token), WebSocket (token in
/// `?accessToken=`), and plain HTTP (`/health`, `/metrics`).
pub struct TcpCluster {
    pub cluster: Cluster,
    pub metrics: Metrics,
    listener: TcpListener,
    conns: BTreeMap<ClientId, Conn>,
    authenticator: Option<Authenticator>,
    next: u32,
    last_tick: Option<Instant>,
}

impl TcpCluster {
    pub fn bind(
        address: &str,
        features: &[Feature],
        zones: &[u32],
        spawn_zone: u32,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            cluster: Cluster::new(features, zones, spawn_zone),
            metrics: Metrics::default(),
            listener,
            conns: BTreeMap::new(),
            authenticator: None,
            next: 1,
            last_tick: None,
        })
    }

    pub fn authenticate_with(&mut self, authenticator: Authenticator) {
        self.authenticator = Some(authenticator);
    }

    pub fn session<T: 'static>(&self, client_id: ClientId) -> Option<&T> {
        self.cluster.session(client_id)
    }

    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.listener
            .local_addr()
            .expect("bound listener has an address")
    }

    pub fn poll(&mut self) {
        while let Ok((stream, _)) = self.listener.accept() {
            stream.set_nonblocking(true).ok();
            stream.set_nodelay(true).ok();
            let client_id = ClientId(self.next);
            self.next += 1;
            self.conns.insert(client_id, Conn::new(stream));
        }
        let Self {
            conns,
            cluster,
            metrics,
            authenticator,
            ..
        } = self;
        let mut dead = Vec::new();
        for (&client_id, conn) in conns.iter_mut() {
            step(conn, client_id, cluster, authenticator, metrics);
            if !conn.ready && conn.opened.elapsed().as_secs() >= HANDSHAKE_TIMEOUT_SECS {
                conn.close.get_or_insert("timeout");
                conn.dead = true;
            }
            if conn.dead {
                dead.push((client_id, conn.close.unwrap_or("normal")));
            }
        }
        for (client_id, reason) in dead {
            let conn = self.conns.remove(&client_id);
            if conn.is_some_and(|conn| conn.ready) {
                self.metrics.connected -= 1;
                self.metrics.close(reason);
                self.cluster.disconnect(client_id);
            }
        }
    }

    pub fn tick(&mut self, delta_time: f32) {
        let started = Instant::now();
        if let Some(last) = self.last_tick {
            self.metrics
                .tick_interval
                .observe(started.duration_since(last).as_secs_f64());
        }
        self.last_tick = Some(started);

        for (client_id, bytes) in self.cluster.tick(delta_time) {
            if let Some(conn) = self.conns.get_mut(&client_id) {
                self.metrics.packets_sent += 1;
                self.metrics.bytes_sent += bytes.len() as u64;
                conn.send_packet(bytes);
            }
        }

        self.metrics.ticks += 1;
        self.metrics.entities = self.cluster.entities() as u64;
        self.metrics
            .tick_duration
            .observe(started.elapsed().as_secs_f64());
    }
}

const HANDSHAKE_TIMEOUT_SECS: u64 = 10;
const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
const MAX_OUTBUF_BYTES: usize = 8 * 1024 * 1024;

fn flush(stream: &mut TcpStream, outbuf: &mut Vec<u8>) -> Result<(), ()> {
    while !outbuf.is_empty() {
        match stream.write(outbuf) {
            Ok(0) => return Err(()),
            Ok(written) => {
                outbuf.drain(..written);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

struct Conn {
    channel: Channel,
    opened: Instant,
    ready: bool,
    dead: bool,
    close: Option<&'static str>,
    session: Option<Session>,
}

enum Channel {
    Pending {
        stream: TcpStream,
        inbuf: Vec<u8>,
        outbuf: Vec<u8>,
        mode: Pending,
    },
    Tcp {
        stream: TcpStream,
        inbuf: Vec<u8>,
        outbuf: Vec<u8>,
    },
    Ws(Box<WebSocket<Replay>>),
    Gone,
}

enum Pending {
    Sniff,
    Head,
    Token,
    Respond,
    // After flushing the 101, the socket goes to the WebSocket codec with any bytes that
    // arrived behind the head.
    Upgrade { leftover: Vec<u8> },
}

impl Conn {
    fn new(stream: TcpStream) -> Self {
        Self {
            channel: Channel::Pending {
                stream,
                inbuf: Vec::new(),
                outbuf: Vec::new(),
                mode: Pending::Sniff,
            },
            opened: Instant::now(),
            ready: false,
            dead: false,
            close: None,
            session: None,
        }
    }

    fn send_packet(&mut self, bytes: Vec<u8>) {
        match &mut self.channel {
            Channel::Tcp { stream, outbuf, .. } => {
                frame(&bytes, outbuf);
                if outbuf.len() > MAX_OUTBUF_BYTES {
                    self.close = Some("error");
                    self.dead = true;
                    return;
                }
                if flush(stream, outbuf).is_err() {
                    self.close = Some("error");
                    self.dead = true;
                }
            }
            Channel::Ws(ws) => match ws.send(Message::binary(bytes)) {
                Ok(()) => {}
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {
                    self.close = Some("error");
                    self.dead = true;
                }
            },
            _ => {}
        }
    }
}

fn step(
    conn: &mut Conn,
    client_id: ClientId,
    cluster: &mut Cluster,
    authenticator: &mut Option<Authenticator>,
    metrics: &mut Metrics,
) {
    if let Channel::Pending { .. } = conn.channel {
        step_pending(conn, client_id, cluster, authenticator, metrics);
    }
    match &mut conn.channel {
        Channel::Tcp {
            stream,
            inbuf,
            outbuf,
        } => {
            let alive = pump(stream, inbuf);
            loop {
                match unframe_one(inbuf) {
                    Ok(Some(packet)) => {
                        metrics.packets_received += 1;
                        metrics.bytes_received += packet.len() as u64;
                        cluster.receive(client_id, &packet);
                    }
                    Ok(None) => break,
                    Err(()) => {
                        conn.close = Some("error");
                        conn.dead = true;
                        break;
                    }
                }
            }
            if flush(stream, outbuf).is_err() {
                conn.close = Some("error");
                conn.dead = true;
            }
            if !alive {
                conn.dead = true;
            }
        }
        Channel::Ws(ws) => {
            loop {
                match ws.read() {
                    Ok(Message::Binary(data)) => {
                        metrics.packets_received += 1;
                        metrics.bytes_received += data.len() as u64;
                        cluster.receive(client_id, &data);
                    }
                    Ok(Message::Close(_)) => {
                        conn.close.get_or_insert("normal");
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                        break;
                    }
                    Err(
                        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed,
                    ) => {
                        conn.close.get_or_insert("normal");
                        conn.dead = true;
                        break;
                    }
                    Err(_) => {
                        conn.close = Some("error");
                        conn.dead = true;
                        break;
                    }
                }
            }
            if !conn.dead {
                match ws.flush() {
                    Ok(())
                    | Err(tungstenite::Error::Io(_))
                    | Err(tungstenite::Error::WriteBufferFull(_)) => {}
                    Err(_) => {
                        conn.close.get_or_insert("normal");
                        conn.dead = true;
                    }
                }
            }
        }
        _ => {}
    }
}

fn step_pending(
    conn: &mut Conn,
    client_id: ClientId,
    cluster: &mut Cluster,
    authenticator: &mut Option<Authenticator>,
    metrics: &mut Metrics,
) {
    let Channel::Pending {
        mut stream,
        mut inbuf,
        mut outbuf,
        mut mode,
    } = std::mem::replace(&mut conn.channel, Channel::Gone)
    else {
        return;
    };

    let alive = pump(&mut stream, &mut inbuf);

    if let Pending::Sniff = mode
        && inbuf.len() >= 4
    {
        mode = if inbuf.starts_with(b"GET ") {
            Pending::Head
        } else {
            Pending::Token
        };
    }

    if let Pending::Head = mode {
        match http::parse_head(&inbuf) {
            Ok(Some(head)) => {
                let leftover = inbuf[head.len..].to_vec();
                if head.upgrade {
                    match (
                        head.ws_key,
                        authenticate(
                            authenticator,
                            &http::query_param(&head.query, "accessToken").unwrap_or_default(),
                        ),
                    ) {
                        (Some(key), Ok(session)) => {
                            outbuf.extend_from_slice(&http::upgrade_response(&derive_accept_key(
                                key.as_bytes(),
                            )));
                            conn.session = session;
                            mode = Pending::Upgrade { leftover };
                        }
                        (None, _) => {
                            outbuf.extend_from_slice(&http::response(
                                400,
                                "Bad Request",
                                "text/plain",
                                b"",
                            ));
                            conn.close = Some("error");
                            mode = Pending::Respond;
                        }
                        (_, Err(_)) => {
                            outbuf.extend_from_slice(&http::response(
                                401,
                                "Unauthorized",
                                "text/plain",
                                b"unauthorized",
                            ));
                            conn.close = Some("unauthorized");
                            mode = Pending::Respond;
                        }
                    }
                } else {
                    let body = match (head.method.as_str(), head.path.as_str()) {
                        ("GET", "/health") => http::response(200, "OK", "text/plain", b"ok"),
                        ("GET", "/metrics") => http::response(
                            200,
                            "OK",
                            "text/plain; version=0.0.4",
                            metrics.render().as_bytes(),
                        ),
                        _ => http::response(404, "Not Found", "text/plain", b""),
                    };
                    outbuf.extend_from_slice(&body);
                    mode = Pending::Respond;
                }
                inbuf.clear();
            }
            Ok(None) => {
                if inbuf.len() > MAX_HANDSHAKE_BYTES {
                    conn.close = Some("error");
                    conn.dead = true;
                }
            }
            Err(()) => {
                outbuf.extend_from_slice(&http::response(400, "Bad Request", "text/plain", b""));
                conn.close = Some("error");
                mode = Pending::Respond;
            }
        }
    }

    if let Pending::Token = mode {
        match unframe_one(&mut inbuf) {
            Ok(Some(token_frame)) => match String::from_utf8(token_frame) {
                Ok(token) => match authenticate(authenticator, &token) {
                    Ok(session) => {
                        conn.session = session;
                        admit(conn, client_id, cluster, metrics);
                        conn.channel = Channel::Tcp {
                            stream,
                            inbuf,
                            outbuf,
                        };
                        return;
                    }
                    Err(_) => {
                        conn.close = Some("unauthorized");
                        conn.dead = true;
                    }
                },
                Err(_) => {
                    conn.close = Some("unauthorized");
                    conn.dead = true;
                }
            },
            Ok(None) => {
                if inbuf.len() > MAX_HANDSHAKE_BYTES {
                    conn.close = Some("error");
                    conn.dead = true;
                }
            }
            Err(()) => {
                conn.close = Some("error");
                conn.dead = true;
            }
        }
    }

    if flush(&mut stream, &mut outbuf).is_err() {
        conn.close.get_or_insert("error");
        conn.dead = true;
    }

    match mode {
        Pending::Respond if outbuf.is_empty() => {
            conn.dead = true;
        }
        Pending::Upgrade { leftover } if outbuf.is_empty() => {
            let config = WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE_BYTES))
                .max_write_buffer_size(MAX_OUTBUF_BYTES);
            let socket = WebSocket::from_raw_socket(
                Replay {
                    prefix: leftover,
                    at: 0,
                    stream,
                },
                Role::Server,
                Some(config),
            );
            admit(conn, client_id, cluster, metrics);
            conn.channel = Channel::Ws(Box::new(socket));
        }
        mode => {
            if !alive {
                conn.dead = true;
            }
            conn.channel = Channel::Pending {
                stream,
                inbuf,
                outbuf,
                mode,
            };
        }
    }
}

fn authenticate(
    authenticator: &mut Option<Authenticator>,
    token: &str,
) -> Result<Option<Session>, String> {
    match authenticator {
        Some(check) => check(token).map(Some),
        None => Ok(None),
    }
}

fn admit(conn: &mut Conn, client_id: ClientId, cluster: &mut Cluster, metrics: &mut Metrics) {
    conn.ready = true;
    metrics.opened += 1;
    metrics.connected += 1;
    cluster.connect_as(client_id, conn.session.take());
}

// Replays bytes already read off the socket before reading from the socket itself.
struct Replay {
    prefix: Vec<u8>,
    at: usize,
    stream: TcpStream,
}

impl Read for Replay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at < self.prefix.len() {
            let take = (self.prefix.len() - self.at).min(buf.len());
            buf[..take].copy_from_slice(&self.prefix[self.at..self.at + take]);
            self.at += take;
            return Ok(take);
        }
        self.stream.read(buf)
    }
}

impl Write for Replay {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}
