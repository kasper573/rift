//! Browser single sign-on every launch. [`sign_in`] runs the OIDC Authorization Code + PKCE flow
//! against a loopback redirect (the testable lib fn the E2E drives); the plugin runs it on a
//! background thread and routes the result into the [`Screen`] states.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use base64::Engine;
use bevy::prelude::*;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, RedirectUrl, Scope,
};

use crate::web;
use crate::{Screen, net};
use world::SPECTATE_ROLE;

/// The public OIDC client id; the game server checks `azp == rift`.
const CLIENT_ID: &str = "rift";

/// What a completed sign-in yields: the access token the game server's `/session` verifies, and
/// the realm roles read (unverified) from it to decide the launch flow.
#[derive(Resource, Clone)]
pub struct Session {
    pub access_token: String,
    pub roles: Vec<String>,
}

pub struct AuthPlugin;

impl Plugin for AuthPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::SigningIn), start)
            .add_systems(Update, poll.run_if(in_state(Screen::SigningIn)));
    }
}

#[derive(Resource)]
struct Pending(Mutex<Receiver<Result<Session, String>>>);

fn start(mut commands: Commands, mut screen: ResMut<NextState<Screen>>) {
    if crate::smoke::enabled() {
        screen.set(Screen::Playing);
        return;
    }
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(sign_in());
    });
    commands.insert_resource(Pending(Mutex::new(rx)));
}

fn poll(
    pending: Option<Res<Pending>>,
    mut commands: Commands,
    mut screen: ResMut<NextState<Screen>>,
) {
    let Some(pending) = pending else {
        return;
    };
    let received = pending.0.lock().expect("pending lock").try_recv();
    match received {
        Ok(Ok(session)) => {
            let spectator = session.roles.iter().any(|role| role == SPECTATE_ROLE);
            commands.insert_resource(session);
            commands.remove_resource::<Pending>();
            screen.set(if spectator {
                Screen::ChooseMode
            } else {
                Screen::Playing
            });
        }
        Ok(Err(error)) => {
            error!("sign-in failed: {error}");
            commands.remove_resource::<Pending>();
            screen.set(Screen::SignInFailed);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            commands.remove_resource::<Pending>();
            screen.set(Screen::SignInFailed);
        }
    }
}

/// Opens the system browser to the realm's authorize endpoint, captures the redirect on the
/// loopback, and exchanges the code for tokens. Blocking; run off the main thread.
pub fn sign_in() -> Result<Session, String> {
    let ca = extra_ca();
    let client_http = web::oidc_client(ca.as_deref());
    let metadata = CoreProviderMetadata::discover(&issuer()?, &client_http).map_err(stringify)?;

    // RFC 8252 loopback redirection: bind any free port on 127.0.0.1 and let the redirect carry it,
    // so two clients (or a stale socket) never collide on a fixed port. The realm registers the
    // port-agnostic `http://127.0.0.1/*`, which Keycloak matches against whatever port we land on.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(stringify)?;
    let redirect = format!("http://{}/", listener.local_addr().map_err(stringify)?);
    let client =
        CoreClient::from_provider_metadata(metadata, ClientId::new(CLIENT_ID.to_owned()), None)
            .set_redirect_uri(RedirectUrl::new(redirect).map_err(stringify)?);

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (authorize_url, csrf, _nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_owned()))
        .set_pkce_challenge(challenge)
        .url();

    webbrowser::open(authorize_url.as_str()).map_err(stringify)?;
    let (code, state) = capture(&listener)?;
    if state != *csrf.secret() {
        return Err("sign-in state mismatch".to_owned());
    }

    let tokens = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(stringify)?
        .set_pkce_verifier(verifier)
        .request(&client_http)
        .map_err(stringify)?;
    let access_token = tokens.access_token().secret().to_owned();
    let roles = roles(&access_token);
    Ok(Session {
        access_token,
        roles,
    })
}

/// Accepts a single redirect request on the loopback, replies with a closing page, and returns the
/// captured `code` and `state`.
fn capture(listener: &TcpListener) -> Result<(String, String), String> {
    let (mut stream, _) = listener.accept().map_err(stringify)?;
    let request = BufReader::new(&stream)
        .lines()
        .next()
        .and_then(Result::ok)
        .ok_or("empty redirect request")?;
    let query = request
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split_once('?'))
        .map(|(_, query)| query.to_owned())
        .unwrap_or_default();
    let mut code = None;
    let mut state = None;
    for (key, value) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        match key {
            "code" => code = Some(value.to_owned()),
            "state" => state = Some(value.to_owned()),
            _ => {}
        }
    }
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nSigned in \xe2\x80\x94 return to the game.",
    );
    match (code, state) {
        (Some(code), Some(state)) => Ok((code, state)),
        _ => Err("redirect missing code or state".to_owned()),
    }
}

/// Reads `realm_access.roles` from the access token's payload — unverified client-side; the game
/// server verifies the token before granting anything.
fn roles(access_token: &str) -> Vec<String> {
    let Some(payload) = access_token.split('.').nth(1) else {
        return Vec::new();
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return Vec::new();
    };
    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    claims["realm_access"]["roles"]
        .as_array()
        .map(|roles| {
            roles
                .iter()
                .filter_map(|role| role.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Connects to the game over netcode using the signed-in session, then announces the chosen mode.
pub fn enter(world: &mut World, spectate: bool) {
    let Some(session) = world.get_resource::<Session>().cloned() else {
        return;
    };
    open(
        world,
        &format!("Bearer {}", session.access_token),
        extra_ca().as_deref(),
        spectate,
    );
}

/// Connects without sign-in for the smoke harness; the local server runs over plain HTTP.
pub fn enter_bypass(world: &mut World) {
    open(world, "Bypass smoke", None, false);
}

/// Mints a session token, opens the netcode connection, and records the join/spectate intent to
/// announce once the connection is welcomed (see [`net::Announce`]).
fn open(world: &mut World, authorization: &str, extra_ca: Option<&[u8]>, spectate: bool) {
    match net::request_token(&game_url(), authorization, extra_ca) {
        Ok(token) => {
            net::connect(world, &token);
            world.insert_resource(net::Announce { spectate });
        }
        Err(error) => error!("could not open a session: {error}"),
    }
}

fn issuer() -> Result<IssuerUrl, String> {
    IssuerUrl::new(env(
        "RIFT_CLIENT_ISSUER",
        option_env!("RIFT_CLIENT_ISSUER"),
        "https://auth.rift.localhost/realms/rift",
    ))
    .map_err(stringify)
}

fn game_url() -> String {
    env(
        "RIFT_CLIENT_GAME_URL",
        option_env!("RIFT_CLIENT_GAME_URL"),
        "https://game-server.rift.localhost",
    )
}

fn extra_ca() -> Option<Vec<u8>> {
    let path = std::env::var("RIFT_CLIENT_EXTRA_CA")
        .ok()
        .or_else(|| option_env!("RIFT_CLIENT_EXTRA_CA").map(str::to_owned))?;
    std::fs::read(path).ok()
}

fn env(var: &str, shipped: Option<&str>, fallback: &str) -> String {
    std::env::var(var)
        .ok()
        .or_else(|| shipped.map(str::to_owned))
        .unwrap_or_else(|| fallback.to_owned())
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}
