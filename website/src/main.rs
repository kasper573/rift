use std::sync::Arc;

use askama::Template;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum_extra::extract::cookie::CookieJar;
use axum_prometheus::PrometheusMetricLayer;
use openidconnect::RedirectUrl;
use openidconnect::url::Url;
use serde::Deserialize;
use tower_http::services::ServeFile;
use tower_http::set_header::SetResponseHeaderLayer;

mod auth;

/// Must match the realm role the game server checks (world::SPECTATE_ROLE).
const SPECTATE_ROLE: &str = "spectate";

/// The `RIFT_WEBSITE_*` environment; auth additionally reads the shared `RIFT_AUTH_*` block.
#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_port")]
    port: u16,
    game_server_url: Url,
    redirect_uri: RedirectUrl,
    wasm: String,
    js_bundle: String,
}

fn default_port() -> u16 {
    80
}

pub struct App {
    pub auth: auth::Auth,
    pub game_server_url: Url,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_WEBSITE_")
        .from_env()
        .expect("RIFT_WEBSITE_* environment");
    let (track, prometheus) = PrometheusMetricLayer::pair();
    metrics_process::Collector::default().describe();
    // Game artifacts change with every deploy: no-cache makes clients revalidate (ServeFile
    // answers conditional requests with 304s) instead of trusting a freshness window.
    let artifacts = axum::Router::new()
        .route_service("/game.wasm", ServeFile::new(&config.wasm))
        .route_service("/mq_js_bundle.js", ServeFile::new(&config.js_bundle))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ));
    let port = config.port;
    let app = Arc::new(App {
        auth: auth::Auth::from_env(config.redirect_uri).await,
        game_server_url: config.game_server_url,
    });
    let router = axum::Router::new()
        .route("/", get(landing))
        .route("/play", get(play))
        .route("/spectate", get(spectate))
        .route("/auth/sign-in", get(auth::sign_in))
        .route("/auth/sign-out", get(auth::sign_out))
        .route("/auth-callback", get(auth::callback))
        .route("/site.css", get(css))
        .merge(artifacts)
        .layer(track)
        .route(
            "/metrics",
            get(move || async move {
                metrics_process::Collector::default().collect();
                prometheus.render()
            }),
        )
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|error| panic!("could not bind 0.0.0.0:{port}: {error}"));
    println!("website listening on 0.0.0.0:{port}");
    axum::serve(listener, router).await.expect("serve");
}

impl App {
    async fn nav(&self, jar: &CookieJar, path: &str) -> Nav {
        let identity = auth::identity(self, jar).await;
        Nav {
            user: identity.as_ref().map(|identity| identity.name.clone()),
            can_spectate: identity
                .is_some_and(|identity| identity.roles.iter().any(|role| role == SPECTATE_ROLE)),
            account_url: self.auth.account_url(),
            path: path.to_owned(),
        }
    }
}

struct Nav {
    user: Option<String>,
    can_spectate: bool,
    account_url: String,
    path: String,
}

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing {
    nav: Nav,
}

#[derive(Template)]
#[template(path = "sign_in_required.html")]
struct SignInRequired {
    nav: Nav,
    action: &'static str,
}

#[derive(Template)]
#[template(path = "denied.html")]
struct Denied {
    nav: Nav,
}

#[derive(Template)]
#[template(path = "game.html")]
struct Game {
    nav: Nav,
}

async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
    })
}

async fn play(State(app): State<Arc<App>>, RawQuery(query): RawQuery, jar: CookieJar) -> Response {
    match auth::identity(&app, &jar).await {
        Some(identity) => game(&app, &jar, "/play", &identity.token, false, &query).await,
        None => page(SignInRequired {
            nav: app.nav(&jar, "/play").await,
            action: "play",
        }),
    }
}

async fn spectate(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
    jar: CookieJar,
) -> Response {
    match auth::identity(&app, &jar).await {
        Some(identity) if identity.roles.iter().any(|role| role == SPECTATE_ROLE) => {
            game(&app, &jar, "/spectate", &identity.token, true, &query).await
        }
        Some(_) => (
            StatusCode::FORBIDDEN,
            page(Denied {
                nav: app.nav(&jar, "/spectate").await,
            }),
        )
            .into_response(),
        None => page(SignInRequired {
            nav: app.nav(&jar, "/spectate").await,
            action: "spectate",
        }),
    }
}

// The wasm client fetches the page-relative `?config` (see the client's platform module): the
// two-line body carries the authenticated ws url and the page's spectate flag.
async fn game(
    app: &App,
    jar: &CookieJar,
    path: &str,
    token: &str,
    spectate: bool,
    query: &Option<String>,
) -> Response {
    if query.as_deref() == Some("config") {
        let mut ws_url = app.game_server_url.clone();
        ws_url.query_pairs_mut().append_pair("accessToken", token);
        return (
            [
                (header::CONTENT_TYPE, "text/plain"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            format!("{ws_url}\n{}", u8::from(spectate)),
        )
            .into_response();
    }
    page(Game {
        nav: app.nav(jar, path).await,
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
