use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use base64::Engine;
use bevy::prelude::*;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, RedirectUrl, Scope,
};

use crate::web;
use crate::{Screen, net};
use world::Role;

/// The public OIDC client id; the game server checks `azp == rift`.
const CLIENT_ID: &str = "rift";

/// How long sign-in waits for the browser to deliver the OAuth redirect before giving up.
const REDIRECT_TIMEOUT: Duration = Duration::from_secs(300);
/// Per-connection read cap, so a silent probe on the loopback port can't stall the wait.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// What entering play requires: the `Bearer <jwt>` Authorization value `/session` accepts, and
/// the realm roles read (unverified) from the token to decide the launch flow.
#[derive(Resource, Clone)]
pub struct Session {
    pub authorization: String,
    pub roles: Vec<Role>,
}

/// The client's endpoints, read once from `RIFT_CLIENT_*` at startup and injected as a resource
/// rather than reached for inside the sign-in flow: `issuer` is the realm the browser authenticates
/// against, and the game server's `/session` lives at `game_server_url`.
#[derive(Resource, Clone, serde::Deserialize)]
pub struct ClientConfig {
    pub issuer: String,
    pub game_server_url: String,
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

fn start(config: Res<ClientConfig>, mut commands: Commands) {
    let issuer = config.issuer.clone();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(sign_in(&issuer));
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
            let spectator = session.roles.contains(&Role::Spectate);
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
fn sign_in(issuer: &str) -> Result<Session, String> {
    let client_http = web::oidc_client();
    let issuer = IssuerUrl::new(issuer.to_owned()).map_err(stringify)?;
    let metadata = CoreProviderMetadata::discover(&issuer, &client_http).map_err(stringify)?;

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

    // Printed so sign-in stays possible when the browser fails to open: visit the URL manually.
    println!("sign in at {authorize_url}");
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
        authorization: format!("Bearer {access_token}"),
        roles,
    })
}

/// Waits on the loopback for the OAuth redirect. The browser also opens speculative preconnects and
/// fetches `/favicon.ico` against the same port, so connections that aren't the redirect are
/// dismissed and the wait continues until the real callback (the one carrying `state`) lands.
fn capture(listener: &TcpListener) -> Result<(String, String), String> {
    let deadline = Instant::now() + REDIRECT_TIMEOUT;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("timed out waiting for the sign-in redirect")?;
        let (mut stream, _) = listener.accept().map_err(stringify)?;
        let _ = stream.set_read_timeout(Some(remaining.min(READ_TIMEOUT)));
        match redirect(&stream) {
            Some(result) => {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
                      <!doctype html><title>Signed in</title>\
                      <p>Signed in! You can close this tab and return to the game.</p>\
                      <script>window.close()</script>",
                );
                return result;
            }
            None => {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
        }
    }
}

/// Parses one loopback request. `None` means it isn't the OAuth redirect (it carries no `state`), so
/// the caller keeps waiting; `Some` carries the authorization code or the realm's reported error.
fn redirect(stream: &TcpStream) -> Option<Result<(String, String), String>> {
    let request = BufReader::new(stream).lines().next()?.ok()?;
    let query = request
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.split_once('?'))
        .map(|(_, query)| query.to_owned())
        .unwrap_or_default();
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        match key {
            "code" => code = Some(value.to_owned()),
            "state" => state = Some(value.to_owned()),
            "error" => error = Some(value.to_owned()),
            _ => {}
        }
    }
    let state = state?;
    Some(match code {
        Some(code) => Ok((code, state)),
        None => Err(error
            .map(|error| format!("sign-in rejected: {error}"))
            .unwrap_or_else(|| "redirect missing code".to_owned())),
    })
}

/// Reads `realm_access.roles` from the access token's payload — unverified client-side; the game
/// server verifies the token before granting anything.
fn roles(access_token: &str) -> Vec<Role> {
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
                .filter_map(|role| role.as_str().and_then(Role::parse))
                .collect()
        })
        .unwrap_or_default()
}

/// Mints a session token with the signed-in session, opens the netcode connection, and records
/// the join/spectate intent to announce once the connection is welcomed (see [`net::Announce`]).
pub fn enter(world: &mut World, spectate: bool) {
    let Some(session) = world.get_resource::<Session>().cloned() else {
        return;
    };
    let game_server_url = world.resource::<ClientConfig>().game_server_url.clone();
    match net::request_token(&game_server_url, &session.authorization) {
        Ok(token) => {
            net::connect(world, &token);
            world.insert_resource(net::Announce { spectate });
        }
        Err(error) => error!("could not open a session: {error}"),
    }
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}
