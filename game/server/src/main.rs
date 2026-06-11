mod auth;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_ecs::lifecycle::Add;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, RepliconChannels};
use bevy_replicon::shared::backend::connected_client::NetworkId;
use bevy_replicon_renet::netcode::{
    ConnectToken, NETCODE_KEY_BYTES, NetcodeServerTransport, ServerAuthentication, ServerConfig,
};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetServer, RepliconRenetPlugins};
use metrics::{counter, gauge, histogram};
use rand::RngCore;
use world::{ClientId, Identity, TICK_HZ};

/// Identifies this game/protocol to netcode; clients minted under a different id cannot connect.
const PROTOCOL_ID: u64 = 0x0072_6966_7400_0001;
/// How long a minted token stays valid before the player must request another.
const TOKEN_EXPIRE_SECS: u64 = 30;
/// How long netcode keeps an idle connection before timing it out.
const CONNECTION_TIMEOUT_SECS: i32 = 15;
const MAX_CLIENTS: usize = 256;

/// The `RIFT_GAME_SERVER_*` environment.
#[derive(serde::Deserialize)]
struct Config {
    #[serde(default = "default_port")]
    port: u16,
    /// The UDP address clients dial (baked into the minted tokens); the bind address with a
    /// loopback host by default, which suits local development and the test stack.
    public_addr: Option<String>,
    #[serde(default)]
    auth_bypass: bool,
}

fn default_port() -> u16 {
    9998
}

fn main() {
    world::validate();
    let config: Config = envy::prefixed("RIFT_GAME_SERVER_")
        .from_env()
        .expect("RIFT_GAME_SERVER_* environment");
    let bind: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("server bind address");
    let public: SocketAddr = match &config.public_addr {
        Some(addr) => addr.parse().expect("RIFT_GAME_SERVER_PUBLIC_ADDR"),
        None => format!("127.0.0.1:{}", config.port)
            .parse()
            .expect("address"),
    };

    let mut private_key = [0u8; NETCODE_KEY_BYTES];
    rand::rng().fill_bytes(&mut private_key);

    let sessions = Sessions::default();
    let http = Http {
        sessions: sessions.clone(),
        verifier: verifier(),
        bypass: config.auth_bypass,
        prometheus: metrics_recorder(),
        private_key,
        public,
    };
    std::thread::spawn(move || serve_http(bind, http));

    simulate(bind, public, private_key, sessions);
}

fn simulate(
    bind: SocketAddr,
    public: SocketAddr,
    private_key: [u8; NETCODE_KEY_BYTES],
    sessions: Sessions,
) {
    let mut app = world::server_app();
    app.add_plugins(RepliconRenetPlugins);

    let channels = app.world().resource::<RepliconChannels>();
    let server = RenetServer::new(ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    });
    let socket =
        UdpSocket::bind(bind).unwrap_or_else(|error| panic!("cannot bind udp {bind}: {error}"));
    let transport = NetcodeServerTransport::new(
        ServerConfig {
            current_time: unix_now(),
            max_clients: MAX_CLIENTS,
            protocol_id: PROTOCOL_ID,
            public_addresses: vec![public],
            authentication: ServerAuthentication::Secure { private_key },
        },
        socket,
    )
    .expect("netcode server transport");

    app.insert_resource(server);
    app.insert_resource(transport);
    app.insert_resource(sessions);
    app.insert_resource(NextClient(1));
    app.add_observer(attach_identity);
    app.finish();

    println!("mmo server listening: netcode udp {bind}, public {public}");

    let mut clients = app
        .world_mut()
        .query_filtered::<(), With<ConnectedClient>>();
    let frame = Duration::from_secs_f32(1.0 / TICK_HZ);
    let mut last_start: Option<Instant> = None;
    loop {
        let started = Instant::now();
        if let Some(last) = last_start.replace(started) {
            histogram!("rift_tick_interval_seconds").record((started - last).as_secs_f64());
        }
        app.update();
        counter!("rift_ticks_total").increment(1);
        histogram!("rift_tick_duration_seconds").record(started.elapsed().as_secs_f64());
        gauge!("rift_clients_connected").set(clients.iter(app.world()).count() as f64);
        gauge!("rift_entities").set(app.world().entities().len() as f64);
        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

/// A fresh sequential game id, assigned to each connection as it lands.
#[derive(Resource)]
struct NextClient(u32);

/// Tokens minted but not yet redeemed, keyed by the netcode client id baked into each, so the
/// connection that lands carries the player it was issued to.
#[derive(Resource, Clone, Default)]
struct Sessions(Arc<Mutex<HashMap<u64, Pending>>>);

struct Pending {
    identity: Identity,
    minted: Instant,
}

impl Sessions {
    fn put(&self, client_id: u64, identity: Identity) {
        let mut map = self.0.lock().expect("sessions lock");
        let now = Instant::now();
        map.retain(|_, pending| {
            now.duration_since(pending.minted) < Duration::from_secs(TOKEN_EXPIRE_SECS + 5)
        });
        map.insert(
            client_id,
            Pending {
                identity,
                minted: now,
            },
        );
    }

    fn take(&self, client_id: u64) -> Option<Identity> {
        self.0
            .lock()
            .expect("sessions lock")
            .remove(&client_id)
            .map(|pending| pending.identity)
    }
}

/// Attaches the connecting client's game id and (if its token carried one) authenticated
/// [`Identity`] as soon as replicon_renet spawns its connection entity, before it can join.
fn attach_identity(
    add: On<Add, NetworkId>,
    network: Query<&NetworkId>,
    sessions: Res<Sessions>,
    mut next: ResMut<NextClient>,
    mut commands: Commands,
) {
    let Ok(network_id) = network.get(add.entity) else {
        return;
    };
    let client = ClientId(next.0);
    next.0 += 1;
    counter!("rift_client_connections_opened_total").increment(1);
    let mut entity = commands.entity(add.entity);
    entity.insert(client);
    if let Some(identity) = sessions.take(network_id.get()) {
        entity.insert(identity);
    }
}

#[derive(Clone)]
struct Http {
    sessions: Sessions,
    verifier: Option<Arc<Mutex<auth::Verifier>>>,
    bypass: bool,
    prometheus: metrics_exporter_prometheus::PrometheusHandle,
    private_key: [u8; NETCODE_KEY_BYTES],
    public: SocketAddr,
}

#[tokio::main(flavor = "current_thread")]
async fn serve_http(addr: SocketAddr, http: Http) {
    let router = axum::Router::new()
        .route("/session", post(session))
        .route("/health", get(health))
        .route("/metrics", get(scrape))
        .with_state(http);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|error| panic!("cannot bind {addr}: {error}"));
    println!("http listening on {addr}");
    // Sessions are transient and nothing persists, so SIGTERM (docker stop, deploys) exits
    // immediately instead of draining live connections.
    tokio::spawn(async {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm handler installs");
        sigterm.recv().await;
        std::process::exit(0);
    });
    axum::serve(listener, router).await.expect("axum serves");
}

/// Verifies a player's `Authorization` and mints a single-use `ConnectToken` for the UDP
/// connection. `Bearer <JWT>` is verified against Keycloak; `Bypass <name>` is an unverified
/// player accepted only when `RIFT_GAME_SERVER_AUTH_BYPASS` is set (local development and tests).
async fn session(State(http): State<Http>, headers: HeaderMap) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let identity = match resolve(&http, authorization) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };

    let client_id = rand::rng().next_u64();
    http.sessions.put(client_id, identity);
    let token = ConnectToken::generate(
        unix_now(),
        PROTOCOL_ID,
        TOKEN_EXPIRE_SECS,
        client_id,
        CONNECTION_TIMEOUT_SECS,
        vec![http.public],
        None,
        &http.private_key,
    )
    .expect("connect token");
    let mut body = Vec::new();
    token.write(&mut body).expect("serialize connect token");
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn resolve(http: &Http, authorization: &str) -> Result<Identity, StatusCode> {
    if let Some(name) = authorization.strip_prefix("Bypass ") {
        if !http.bypass {
            return Err(StatusCode::UNAUTHORIZED);
        }
        // Bypass players are ordinary; privileged roles require a real token.
        return Ok(Identity {
            id: format!("bypass:{name}"),
            name: name.to_owned(),
            roles: Vec::new(),
        });
    }
    if let Some(token) = authorization.strip_prefix("Bearer ") {
        let verifier = http.verifier.as_ref().ok_or(StatusCode::UNAUTHORIZED)?;
        let claims = verifier
            .lock()
            .expect("verifier lock")
            .verify(token)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        return Ok(Identity {
            id: claims.subject,
            name: claims.name,
            roles: claims.roles,
        });
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn scrape(State(http): State<Http>) -> String {
    metrics_process::Collector::default().collect();
    http.prometheus.render()
}

/// Healthy means players can actually connect: a server that cannot verify tokens yet (issuer
/// still booting next to it) stays out of rotation instead of minting unusable sessions.
async fn health(State(http): State<Http>) -> Response {
    let ready = match &http.verifier {
        Some(verifier) => verifier.lock().expect("verifier lock").ready(),
        None => true,
    };
    if ready {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "auth keys unavailable").into_response()
    }
}

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after the unix epoch")
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

/// Keycloak verification configured through the shared `RIFT_AUTH_*` block; without it the server
/// mints sessions for anyone (plain local development).
fn verifier() -> Option<Arc<Mutex<auth::Verifier>>> {
    #[derive(serde::Deserialize)]
    struct AuthConfig {
        issuer: String,
        audience: String,
        jwks_uri: String,
    }
    let config: AuthConfig = envy::prefixed("RIFT_AUTH_").from_env().ok()?;
    let mut verifier = auth::Verifier::new(&config.issuer, &config.audience, &config.jwks_uri);
    match verifier.warm() {
        Ok(()) => println!("auth enabled, issuer {}", config.issuer),
        Err(error) => println!(
            "auth enabled, issuer {} (jwks warm-up failed: {error})",
            config.issuer
        ),
    }
    Some(Arc::new(Mutex::new(verifier)))
}
