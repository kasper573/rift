mod auth;
mod pages;

pub use auth::SignInSettings;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use openidconnect::RedirectUrl;
use serde::{Deserialize, Serialize};
use service::GameInfo;
use tower_http::services::ServeDir;

pub const ENV_PREFIX: &str = "RIFT_WEB_SERVER_";

/// The prefix of the sign-in settings the site hands desktop clients in their `rift.env`.
pub const DESKTOP_SIGN_IN_ENV_PREFIX: &str = "RIFT_WEB_SERVER_DESKTOP_AUTH_";

#[derive(Serialize, Deserialize)]
pub struct WebServerConfig {
    pub bind: IpAddr,
    pub port: u16,
    pub redirect_uri: RedirectUrl,
    /// The game server as browsers and desktop clients reach it.
    pub game_server_url: String,
    pub game_server_ws_url: String,
    /// The same game server as the web-server itself reaches it.
    pub game_server_internal_url: String,
    /// Where releases are downloaded from: a release's archives are at `<downloads_url>/<release>/`.
    pub downloads_url: String,
    /// The engine release the site serves, whose desktop client it offers.
    pub release: String,
    pub wasm_dir: PathBuf,
}

/// Everything the site is made from.
pub struct Site {
    pub config: WebServerConfig,
    pub sign_in: SignInSettings,
    pub desktop_sign_in: oidc::BrowserSignInSettings,
    pub game: GameInfo,
}

pub async fn router(site: Site) -> Router {
    let Site {
        config,
        sign_in,
        desktop_sign_in,
        game,
    } = site;
    let domain = config
        .redirect_uri
        .url()
        .host_str()
        .expect("a redirect uri with a host")
        .to_owned();
    let wasm_dir = config.wasm_dir.clone();
    let app = Arc::new(App {
        auth: auth::Auth::new(sign_in, config.redirect_uri.clone()).await,
        domain,
        game,
        desktop_sign_in,
        config,
    });
    Router::new()
        .route("/", get(pages::landing))
        .route("/play", get(pages::play))
        .route("/downloads", get(pages::downloads))
        .route("/rift.env", get(pages::rift_env))
        .route("/auth/sign-in", get(auth::sign_in))
        .route("/auth/sign-out", get(auth::sign_out))
        .route("/auth-callback", get(auth::callback))
        .route("/site.css", get(pages::css))
        .route("/favicon.png", get(pages::favicon))
        .nest_service("/wasm", ServeDir::new(wasm_dir))
        .with_state(app)
}

/// What the game server publishes about its game, waiting for the server to answer.
pub async fn published_game(game_server_url: &str) -> GameInfo {
    let url = format!("{game_server_url}{}", service::GAME_INFO_ROUTE);
    let http = openidconnect::reqwest::Client::new();
    loop {
        match fetch_game(&http, &url).await {
            Ok(game) => return game,
            Err(error) => {
                println!("waiting for the game server at {url}: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

pub(crate) struct App {
    pub(crate) auth: auth::Auth,
    pub(crate) domain: String,
    pub(crate) game: GameInfo,
    pub(crate) desktop_sign_in: oidc::BrowserSignInSettings,
    pub(crate) config: WebServerConfig,
}

async fn fetch_game(http: &openidconnect::reqwest::Client, url: &str) -> Result<GameInfo, String> {
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|error| error.to_string())?;
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}
