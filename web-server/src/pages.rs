use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;

use crate::{App, auth};

pub(crate) async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
    })
}

pub(crate) async fn play(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    let nav = app.nav(&jar, "/play").await;
    let access_token = nav
        .user
        .as_ref()
        .and_then(|_| jar.get("token").map(|cookie| cookie.value().to_owned()));
    page(Play {
        nav,
        access_token,
        game_server_url: app.config.game_server_url.clone(),
        game_server_ws_url: app.config.game_server_ws_url.clone(),
    })
}

pub(crate) async fn downloads(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    let platforms = PLATFORMS
        .iter()
        .map(|platform| PlatformDownload {
            platform,
            client: format!(
                "{}/{}/{}",
                app.config.downloads_url,
                app.config.release,
                platform.archive(CLIENT)
            ),
            archive: platform.archive(CLIENT),
        })
        .collect();
    page(Downloads {
        nav: app.nav(&jar, "/downloads").await,
        platforms,
        domain: app.domain.clone(),
    })
}

pub(crate) async fn rift_env(State(app): State<Arc<App>>) -> Response {
    let file = RiftEnv {
        domain: &app.domain,
        game_server_url: &app.config.game_server_url,
        game_server_ws_url: &app.config.game_server_ws_url,
        sign_in: &app.desktop_sign_in,
    };
    match file.render() {
        Ok(text) => (
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"rift.env\"",
                ),
            ],
            text,
        )
            .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(crate) async fn favicon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        include_bytes!("../static/favicon.png").as_slice(),
    )
}

pub(crate) async fn css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/site.css"),
    )
}

impl App {
    async fn nav(&self, jar: &CookieJar, path: &str) -> Nav {
        let identity = auth::identity(self, jar).await;
        Nav {
            game: self.game.name.clone(),
            user: identity.map(|identity| identity.name),
            account_url: self.auth.account_url(),
            path: path.to_owned(),
        }
    }
}

struct Nav {
    game: String,
    user: Option<String>,
    account_url: Option<String>,
    path: String,
}

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing {
    nav: Nav,
}

#[derive(Template)]
#[template(path = "play.html")]
struct Play {
    nav: Nav,
    access_token: Option<String>,
    game_server_url: String,
    game_server_ws_url: String,
}

#[derive(Template)]
#[template(path = "downloads.html")]
struct Downloads {
    nav: Nav,
    platforms: Vec<PlatformDownload>,
    domain: String,
}

#[derive(Template)]
#[template(path = "rift.env", escape = "none")]
struct RiftEnv<'a> {
    domain: &'a str,
    game_server_url: &'a str,
    game_server_ws_url: &'a str,
    sign_in: &'a oidc::BrowserSignInSettings,
}

struct PlatformDownload {
    platform: &'static Platform,
    archive: String,
    client: String,
}

struct Platform {
    id: &'static str,
    name: &'static str,
    detail: &'static str,
    archive_suffix: &'static str,
}

impl Platform {
    /// The name of the release's archive of `binary` for this platform.
    fn archive(&self, binary: &str) -> String {
        format!("{binary}-{}{}", self.id, self.archive_suffix)
    }
}

const CLIENT: &str = "desktop-client";

const PLATFORMS: [Platform; 3] = [
    Platform {
        id: "linux-x86_64",
        name: "Linux",
        detail: "x86-64",
        archive_suffix: ".tar.gz",
    },
    Platform {
        id: "windows-x86_64",
        name: "Windows",
        detail: "x86-64",
        archive_suffix: ".zip",
    },
    Platform {
        id: "macos-arm64",
        name: "macOS",
        detail: "Apple silicon",
        archive_suffix: ".tar.gz",
    },
];

fn page<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}
