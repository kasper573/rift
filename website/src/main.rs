use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum_extra::extract::cookie::CookieJar;
use axum_prometheus::PrometheusMetricLayer;
use openidconnect::RedirectUrl;
use pyroscope::backend::{BackendConfig, PprofConfig, pprof_backend};
use pyroscope::pyroscope::{PyroscopeAgent, PyroscopeAgentBuilder, PyroscopeAgentRunning};
use serde::Deserialize;

mod auth;

/// The `RIFT_WEBSITE_*` environment; auth additionally reads the shared `RIFT_AUTH_*` block.
#[derive(Deserialize)]
struct Config {
    port: u16,
    redirect_uri: RedirectUrl,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
}

#[derive(Deserialize)]
struct Installers {
    installer_links: Vec<String>,
}

pub struct App {
    pub auth: auth::Auth,
    downloads: Vec<Download>,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_WEBSITE_")
        .from_env()
        .expect("RIFT_WEBSITE_* environment");
    let installers: Installers = envy::prefixed("RIFT_")
        .from_env()
        .expect("RIFT_INSTALLER_LINKS environment");
    // Held for the process lifetime: dropping the agent stops the profiler.
    let _profiler = if config.pyroscope_enabled {
        Some(start_profiler("rift-website", config.pyroscope_sample_hz))
    } else {
        None
    };
    let (track, prometheus) = PrometheusMetricLayer::pair();
    metrics_process::Collector::default().describe();
    let port = config.port;
    let app = Arc::new(App {
        auth: auth::Auth::from_env(config.redirect_uri).await,
        downloads: installers
            .installer_links
            .iter()
            .map(|url| Download {
                filename: filename_of(url),
                url: url.clone(),
            })
            .collect(),
    });
    let router = axum::Router::new()
        .route("/", get(landing))
        .route("/downloads", get(downloads))
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
}

async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
    })
}

#[derive(Clone)]
struct Download {
    filename: String,
    url: String,
}

#[derive(Template)]
#[template(path = "downloads.html")]
struct Downloads {
    nav: Nav,
    downloads: Vec<Download>,
}

async fn downloads(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Downloads {
        nav: app.nav(&jar, "/downloads").await,
        downloads: app.downloads.clone(),
    })
}

/// Continuously samples this process at `sample_hz` and pushes profiles to the `PYROSCOPE_BASE`
/// server under the given application name, where they surface in Grafana's profiles drilldown.
/// The returned agent must be held for the process lifetime, as dropping it stops profiling.
fn start_profiler(application: &str, sample_hz: u32) -> PyroscopeAgent<PyroscopeAgentRunning> {
    #[derive(Deserialize)]
    struct Profiling {
        base: String,
    }
    let profiling: Profiling = envy::prefixed("PYROSCOPE_")
        .from_env()
        .expect("PYROSCOPE_* environment");
    PyroscopeAgentBuilder::new(
        profiling.base,
        application,
        sample_hz,
        "pyroscope-rs",
        env!("CARGO_PKG_VERSION"),
        pprof_backend(
            PprofConfig {
                sample_rate: sample_hz,
            },
            BackendConfig::default(),
        ),
    )
    .build()
    .expect("build pyroscope agent")
    .start()
    .expect("start pyroscope agent")
}

/// The label shown for a download link: the URL's last path segment.
fn filename_of(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_owned()
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
