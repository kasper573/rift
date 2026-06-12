use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum_extra::extract::cookie::CookieJar;
use axum_prometheus::PrometheusMetricLayer;
use openidconnect::RedirectUrl;
use serde::Deserialize;

mod auth;

/// The `RIFT_WEBSITE_*` environment; auth additionally reads the shared `RIFT_AUTH_*` block.
#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_port")]
    port: u16,
    redirect_uri: RedirectUrl,
    /// The GitHub `owner/name` whose releases the landing page's download link points at.
    #[serde(default = "default_repo")]
    repo: String,
}

fn default_port() -> u16 {
    80
}

fn default_repo() -> String {
    "kasper573/rift".to_owned()
}

pub struct App {
    pub auth: auth::Auth,
    releases_url: String,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_WEBSITE_")
        .from_env()
        .expect("RIFT_WEBSITE_* environment");
    let (track, prometheus) = PrometheusMetricLayer::pair();
    metrics_process::Collector::default().describe();
    let port = config.port;
    let app = Arc::new(App {
        auth: auth::Auth::from_env(config.redirect_uri).await,
        releases_url: format!("https://github.com/{}/releases/latest", config.repo),
    });
    let router = axum::Router::new()
        .route("/", get(landing))
        .route("/auth/sign-in", get(auth::sign_in))
        .route("/auth/sign-out", get(auth::sign_out))
        .route("/auth-callback", get(auth::callback))
        .route("/site.css", get(css))
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
    // Drain in-flight requests on a stop request (docker stop, deploys). ctrlc handles the platform
    // signals; a Notify bridges its callback into the async shutdown axum awaits.
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.notify_one()).expect("install stop handler");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { stop.notified().await })
        .await
        .expect("serve");
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
    releases_url: String,
}

async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
        releases_url: app.releases_url.clone(),
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
