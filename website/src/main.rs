use std::path::PathBuf;
use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum_extra::extract::cookie::CookieJar;
use openidconnect::RedirectUrl;
use serde::Deserialize;
use tower_http::services::ServeDir;

mod auth;

service::heap_profiling!();

#[derive(Deserialize)]
struct Config {
    port: u16,
    redirect_uri: RedirectUrl,
    /// The https:// game-server origin the embedded client posts to for a session, injected into `/play`.
    game_server_url: String,
    /// The wss:// game-server origin the embedded client dials for netcode, injected into `/play`.
    game_server_ws_url: String,
    /// Directory of the wasm client bundle, served at `/wasm` (baked into the image by `just wasm`).
    wasm_dir: PathBuf,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
}

pub struct App {
    pub auth: auth::Auth,
    game_server_url: String,
    game_server_ws_url: String,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_WEBSITE_")
        .from_env()
        .expect("RIFT_WEBSITE_* environment");
    let _profiler = service::profiler(
        "rift-website",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
    let port = config.port;
    let app = Arc::new(App {
        auth: auth::Auth::from_env(config.redirect_uri).await,
        game_server_url: config.game_server_url,
        game_server_ws_url: config.game_server_ws_url,
    });
    let router = axum::Router::new()
        .route("/", get(landing))
        .route("/play", get(play))
        .route("/auth/sign-in", get(auth::sign_in))
        .route("/auth/sign-out", get(auth::sign_out))
        .route("/auth-callback", get(auth::callback))
        .route("/site.css", get(css))
        .nest_service("/wasm", ServeDir::new(config.wasm_dir))
        .with_state(app);
    service::serve("website", port, router).await;
}

impl App {
    async fn nav(&self, jar: &CookieJar, path: &str) -> Nav {
        let identity = auth::identity(self, jar).await;
        Nav {
            user: identity.map(|identity| identity.name),
            account_url: self.auth.account_url(),
            path: path.to_owned(),
        }
    }
}

struct Nav {
    user: Option<String>,
    account_url: String,
    path: String,
}

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing {
    nav: Nav,
}

async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
    })
}

#[derive(Template)]
#[template(path = "play.html")]
struct Play {
    nav: Nav,
    access_token: Option<String>,
    game_server_url: String,
    game_server_ws_url: String,
}

async fn play(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    let nav = app.nav(&jar, "/play").await;
    let access_token = nav
        .user
        .as_ref()
        .and_then(|_| jar.get("token").map(|cookie| cookie.value().to_owned()));
    page(Play {
        nav,
        access_token,
        game_server_url: app.game_server_url.clone(),
        game_server_ws_url: app.game_server_ws_url.clone(),
    })
}

fn page<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/site.css"),
    )
}
