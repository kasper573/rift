use std::sync::{Arc, Mutex};
use std::time::Duration;

use askama::Template;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use tower_http::services::ServeFile;
use tower_http::set_header::SetResponseHeaderLayer;

mod oidc;

/// Must match the realm role the game server checks (world::SPECTATE_ROLE).
const SPECTATE_ROLE: &str = "spectate";

pub struct App {
    pub authority: String,
    pub audience: String,
    pub redirect_uri: String,
    pub token_uri: String,
    pub game_server_url: String,
    pub wasm_path: String,
    pub bundle_path: String,
    pub verifier: Mutex<auth::Verifier>,
    pub http: ureq::Agent,
}

#[tokio::main]
async fn main() {
    let app = Arc::new(App::from_env());
    let port = std::env::var("RIFT_WEBSITE_PORT").unwrap_or_else(|_| "80".to_owned());
    // Game artifacts change with every deploy: no-cache makes clients revalidate (ServeFile
    // answers conditional requests with 304s) instead of trusting a freshness window.
    let artifacts = axum::Router::new()
        .route_service("/game.wasm", ServeFile::new(&app.wasm_path))
        .route_service("/mq_js_bundle.js", ServeFile::new(&app.bundle_path))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ));
    let router = axum::Router::new()
        .route("/", get(landing))
        .route("/play", get(play))
        .route("/spectate", get(spectate))
        .route("/auth/sign-in", get(oidc::sign_in))
        .route("/auth/sign-out", get(oidc::sign_out))
        .route("/auth-callback", get(oidc::callback))
        .route("/site.css", get(css))
        .merge(artifacts)
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|error| panic!("could not bind 0.0.0.0:{port}: {error}"));
    println!("website listening on 0.0.0.0:{port}");
    axum::serve(listener, router).await.expect("serve");
}

impl App {
    fn from_env() -> App {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| panic!("{name} must be set"))
        };
        let authority = var("RIFT_WEBSITE_AUTH__AUTHORITY");
        let audience = var("RIFT_WEBSITE_AUTH__AUDIENCE");
        let mut verifier = auth::Verifier::new(
            &authority,
            &audience,
            &var("RIFT_WEBSITE_AUTH__JWKS_URI"),
            false,
        );
        match verifier.warm() {
            Ok(()) => println!("auth ready, issuer {authority}"),
            Err(error) => println!("auth ready, issuer {authority} (jwks warm-up failed: {error})"),
        }
        App {
            redirect_uri: var("RIFT_WEBSITE_AUTH__REDIRECT_URI"),
            token_uri: var("RIFT_WEBSITE_AUTH__TOKEN_URI"),
            game_server_url: var("RIFT_WEBSITE_GAME_SERVER_URL"),
            wasm_path: var("RIFT_GAME_CLIENT_WASM"),
            bundle_path: var("RIFT_MQ_JS_BUNDLE"),
            authority,
            audience,
            verifier: Mutex::new(verifier),
            http: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(5)))
                    .build(),
            ),
        }
    }

    pub fn identity(&self, headers: &HeaderMap) -> Option<(auth::Claims, String)> {
        let token = oidc::cookie(headers, "token")?;
        let claims = self.verifier.lock().ok()?.verify(&token).ok()?;
        Some((claims, token))
    }

    fn nav(&self, headers: &HeaderMap, path: &str) -> Nav {
        let identity = self.identity(headers);
        Nav {
            user: identity.as_ref().map(|(claims, _)| claims.name.clone()),
            can_spectate: identity
                .is_some_and(|(claims, _)| claims.roles.iter().any(|role| role == SPECTATE_ROLE)),
            account_url: format!("{}/account", self.authority),
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

async fn landing(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    page(Landing {
        nav: app.nav(&headers, "/"),
    })
}

async fn play(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    match app.identity(&headers) {
        Some((_, token)) => game(&app, &headers, "/play", &token, false, &query),
        None => page(SignInRequired {
            nav: app.nav(&headers, "/play"),
            action: "play",
        }),
    }
}

async fn spectate(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    match app.identity(&headers) {
        Some((claims, token)) if claims.roles.iter().any(|role| role == SPECTATE_ROLE) => {
            game(&app, &headers, "/spectate", &token, true, &query)
        }
        Some(_) => (
            StatusCode::FORBIDDEN,
            page(Denied {
                nav: app.nav(&headers, "/spectate"),
            }),
        )
            .into_response(),
        None => page(SignInRequired {
            nav: app.nav(&headers, "/spectate"),
            action: "spectate",
        }),
    }
}

// The wasm client fetches the page-relative `?config` (see the client's platform module): the
// two-line body carries the authenticated ws url and the page's spectate flag.
fn game(
    app: &App,
    headers: &HeaderMap,
    path: &str,
    token: &str,
    spectate: bool,
    query: &Option<String>,
) -> Response {
    if query.as_deref() == Some("config") {
        let ws_url = format!(
            "{}?accessToken={}",
            app.game_server_url,
            oidc::urlencode(token)
        );
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
        nav: app.nav(headers, path),
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
