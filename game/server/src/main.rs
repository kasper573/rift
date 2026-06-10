//! The composition root: terminates WebSockets with axum, shuttles byte frames between
//! connections and the `world` simulation, ticks it at [`world::TICK_HZ`], and serves the
//! `/health` and `/metrics` endpoints the stack scrapes.

mod auth;

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics::{counter, gauge, histogram};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use world::{ClientId, ConnectedClient, Entity, Identity, ServerMessages, TICK_HZ};

const MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// The `RIFT_GAME_SERVER_*` environment; without a port the address falls back to the first
/// argument, then [`world::DEFAULT_ADDRESS`].
#[derive(serde::Deserialize)]
struct Config {
    hostname: Option<String>,
    port: Option<u16>,
    #[serde(default)]
    auth_bypass: bool,
}

fn main() {
    world::validate();
    let config: Config = envy::prefixed("RIFT_GAME_SERVER_")
        .from_env()
        .expect("RIFT_GAME_SERVER_* environment");
    let address = config
        .port
        .map(|port| format!("{}:{port}", config.hostname.as_deref().unwrap_or("0.0.0.0")))
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_else(|| world::DEFAULT_ADDRESS.to_owned());

    let (events_tx, events) = mpsc::channel();
    let net = Net {
        events: events_tx,
        verifier: verifier(config.auth_bypass),
        prometheus: metrics_recorder(),
    };
    std::thread::spawn(move || serve(address, net));
    simulate(events);
}

enum Event {
    Connected {
        identity: Option<Identity>,
        outbound: UnboundedSender<Vec<u8>>,
        entity: oneshot::Sender<Entity>,
    },
    Frame {
        entity: Entity,
        bytes: Vec<u8>,
    },
    Closed {
        entity: Entity,
    },
}

/// The fixed-rate authoritative loop: drains transport events into the world, updates it, and
/// fans replication out to each connection's outbox.
fn simulate(events: mpsc::Receiver<Event>) {
    let mut app = world::server_app();
    let mut outboxes: HashMap<Entity, UnboundedSender<Vec<u8>>> = HashMap::new();
    let mut next_client = 1u32;
    let frame = Duration::from_secs_f32(1.0 / TICK_HZ);
    let mut last_start: Option<Instant> = None;
    loop {
        let started = Instant::now();
        if let Some(last) = last_start.replace(started) {
            histogram!("rift_tick_interval_seconds").record((started - last).as_secs_f64());
        }

        for event in events.try_iter() {
            match event {
                Event::Connected {
                    identity,
                    outbound,
                    entity,
                } => {
                    let client = ClientId(next_client);
                    next_client += 1;
                    let mut spawned = app.world_mut().spawn((
                        ConnectedClient {
                            max_size: MAX_MESSAGE_SIZE,
                        },
                        client,
                    ));
                    if let Some(identity) = identity {
                        spawned.insert(identity);
                    }
                    outboxes.insert(spawned.id(), outbound);
                    let _ = entity.send(spawned.id());
                }
                Event::Frame { entity, bytes } => {
                    if outboxes.contains_key(&entity)
                        && let Some((&channel, payload)) = bytes.split_first()
                    {
                        app.world_mut()
                            .resource_mut::<ServerMessages>()
                            .insert_received(entity, channel as usize, payload.to_vec());
                    }
                }
                Event::Closed { entity } => {
                    if outboxes.remove(&entity).is_some() {
                        app.world_mut().despawn(entity);
                    }
                }
            }
        }

        app.update();

        let sent: Vec<_> = app
            .world_mut()
            .resource_mut::<ServerMessages>()
            .drain_sent()
            .collect();
        for (entity, channel, payload) in sent {
            if let Some(outbox) = outboxes.get(&entity) {
                let mut frame = Vec::with_capacity(payload.len() + 1);
                frame.push(u8::try_from(channel).expect("replicon channels fit a byte"));
                frame.extend_from_slice(&payload);
                let _ = outbox.send(frame);
            }
        }

        counter!("rift_ticks_total").increment(1);
        histogram!("rift_tick_duration_seconds").record(started.elapsed().as_secs_f64());
        gauge!("rift_clients_connected").set(outboxes.len() as f64);
        gauge!("rift_entities").set(app.world().entities().len() as f64);
        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

#[derive(Clone)]
struct Net {
    events: mpsc::Sender<Event>,
    verifier: Option<std::sync::Arc<std::sync::Mutex<auth::Verifier>>>,
    prometheus: metrics_exporter_prometheus::PrometheusHandle,
}

#[tokio::main(flavor = "current_thread")]
async fn serve(address: String, net: Net) {
    let app = axum::Router::new()
        .route("/ws", get(upgrade))
        .route("/health", get(health))
        .route("/metrics", get(scrape))
        .with_state(net);
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .unwrap_or_else(|error| panic!("cannot bind {address}: {error}"));
    println!("mmo server listening on {address}");
    // Sessions are transient and nothing persists, so SIGTERM (docker stop, deploys) exits
    // immediately instead of draining the long-lived WebSockets.
    tokio::spawn(async {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm handler installs");
        sigterm.recv().await;
        std::process::exit(0);
    });
    axum::serve(listener, app).await.expect("axum serves");
}

async fn scrape(State(net): State<Net>) -> String {
    metrics_process::Collector::default().collect();
    net.prometheus.render()
}

/// Healthy means players can actually connect: a server that cannot verify tokens yet (issuer
/// still booting next to it) stays out of rotation instead of answering upgrades with 401s.
async fn health(State(net): State<Net>) -> Response {
    let ready = match &net.verifier {
        Some(verifier) => verifier.lock().expect("verifier lock").ready(),
        None => true,
    };
    if ready {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "auth keys unavailable").into_response()
    }
}

async fn upgrade(
    State(net): State<Net>,
    Query(params): Query<HashMap<String, String>>,
    request: WebSocketUpgrade,
) -> Response {
    let token = params.get("accessToken").cloned().unwrap_or_default();
    let identity = match &net.verifier {
        Some(verifier) => {
            let verified = verifier.lock().expect("verifier lock").verify(&token);
            match verified {
                Ok(claims) => Some(Identity {
                    id: claims.subject,
                    name: claims.name,
                    roles: claims.roles,
                }),
                Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
            }
        }
        None => None,
    };
    request.on_upgrade(move |socket| connection(net, socket, identity))
}

async fn connection(net: Net, mut socket: WebSocket, identity: Option<Identity>) {
    let (outbound, mut frames) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (entity_tx, entity_rx) = oneshot::channel();
    if net
        .events
        .send(Event::Connected {
            identity,
            outbound,
            entity: entity_tx,
        })
        .is_err()
    {
        return;
    }
    let Ok(entity) = entity_rx.await else {
        return;
    };
    counter!("rift_client_connections_opened_total").increment(1);

    let code = loop {
        tokio::select! {
            frame = frames.recv() => match frame {
                Some(bytes) => {
                    counter!("rift_bytes_sent_total").increment(bytes.len() as u64);
                    counter!("rift_packets_sent_total").increment(1);
                    if socket.send(Message::Binary(bytes.into())).await.is_err() {
                        break "send_error";
                    }
                }
                None => break "server_closed",
            },
            received = socket.recv() => match received {
                Some(Ok(Message::Binary(bytes))) => {
                    counter!("rift_bytes_received_total").increment(bytes.len() as u64);
                    counter!("rift_packets_received_total").increment(1);
                    let _ = net.events.send(Event::Frame { entity, bytes: bytes.into() });
                }
                Some(Ok(Message::Close(_))) | None => break "closed",
                Some(Ok(_)) => {}
                Some(Err(_)) => break "receive_error",
            },
        }
    };
    counter!("rift_client_connections_closed_total", "code" => code).increment(1);
    let _ = net.events.send(Event::Closed { entity });
}

fn metrics_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    metrics_process::Collector::default().describe();
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(&[
            0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
        ])
        .expect("non-empty histogram buckets")
        .install_recorder()
        .expect("prometheus recorder installs")
}

/// Keycloak verification configured through the shared `RIFT_AUTH_*` block; without it the
/// server runs open (plain local development).
fn verifier(allow_bypass: bool) -> Option<std::sync::Arc<std::sync::Mutex<auth::Verifier>>> {
    #[derive(serde::Deserialize)]
    struct AuthConfig {
        issuer: String,
        audience: String,
        jwks_uri: String,
    }
    let config: AuthConfig = envy::prefixed("RIFT_AUTH_").from_env().ok()?;
    let mut verifier = auth::Verifier::new(
        &config.issuer,
        &config.audience,
        &config.jwks_uri,
        allow_bypass,
    );
    match verifier.warm() {
        Ok(()) => println!("auth enabled, issuer {}", config.issuer),
        Err(error) => println!(
            "auth enabled, issuer {} (jwks warm-up failed: {error})",
            config.issuer
        ),
    }
    Some(std::sync::Arc::new(std::sync::Mutex::new(verifier)))
}
