use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum_extra::extract::cookie::CookieJar;
use openidconnect::RedirectUrl;
use serde::Deserialize;

mod auth;

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
    let _profiler = service::profiler(
        "rift-website",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
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
