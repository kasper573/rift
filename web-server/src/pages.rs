use std::sync::Arc;

use askama::Template;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;

use crate::bundle::{self, ArchiveKind};
use crate::{App, auth};

pub(crate) async fn landing(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Landing {
        nav: app.nav(&jar, "/").await,
    })
}

pub(crate) async fn downloads(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    page(Downloads {
        nav: app.nav(&jar, "/downloads").await,
        platforms: &PLATFORMS,
    })
}

/// The game client of `platform`, extracted to a folder that holds everything it needs to play
/// on this site.
pub(crate) async fn download(
    State(app): State<Arc<App>>,
    Path(platform): Path<String>,
) -> Response {
    let Some(index) = PLATFORMS.iter().position(|known| known.id == platform) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let platform = &PLATFORMS[index];
    let bundle = app.bundles[index]
        .get_or_try_init(|| app.bundle(platform))
        .await;
    match bundle {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", app.archive_name(platform)),
                ),
            ],
            bytes.clone(),
        )
            .into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, error).into_response(),
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
#[template(path = "downloads.html")]
struct Downloads {
    nav: Nav,
    platforms: &'static [Platform],
}

#[derive(Template)]
#[template(path = "rift.env", escape = "none")]
struct RiftEnv<'a> {
    domain: &'a str,
    game_server_url: &'a str,
    releases_url: &'a str,
    sign_in: &'a oidc::BrowserSignInSettings,
}

pub(crate) struct Platform {
    id: &'static str,
    name: &'static str,
    detail: &'static str,
    kind: ArchiveKind,
}

impl Platform {
    fn suffix(&self) -> &'static str {
        match self.kind {
            ArchiveKind::TarGz => ".tar.gz",
            ArchiveKind::Zip => ".zip",
        }
    }
}

impl App {
    async fn bundle(&self, platform: &Platform) -> Result<Bytes, String> {
        let url = format!(
            "{}/{}/{CLIENT}-{}{}",
            self.config.downloads_url,
            self.config.release,
            platform.id,
            platform.suffix()
        );
        let fetched = async {
            openidconnect::reqwest::Client::new()
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await
        };
        let release = fetched.await.map_err(|error| format!("{url}: {error}"))?;
        let rift_env = RiftEnv {
            domain: &self.domain,
            game_server_url: &self.config.game_server_url,
            releases_url: &self.config.downloads_url,
            sign_in: &self.desktop_sign_in,
        }
        .render()
        .map_err(|error| error.to_string())?;
        bundle::bundle(&release, platform.kind, &self.folder_name(), &rift_env).map(Bytes::from)
    }

    /// The folder the game client extracts to, named after the game.
    fn folder_name(&self) -> String {
        let named: String = self
            .game
            .name
            .chars()
            .map(|letter| {
                if letter.is_alphanumeric() {
                    letter
                } else {
                    '-'
                }
            })
            .collect();
        let named = named
            .split('-')
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if named.is_empty() {
            CLIENT.to_owned()
        } else {
            named
        }
    }

    fn archive_name(&self, platform: &Platform) -> String {
        format!(
            "{}-{}{}",
            self.folder_name(),
            platform.id,
            platform.suffix()
        )
    }
}

const CLIENT: &str = "game-client";

pub(crate) const PLATFORMS: [Platform; 3] = [
    Platform {
        id: "linux-x86_64",
        name: "Linux",
        detail: "x86-64",
        kind: ArchiveKind::TarGz,
    },
    Platform {
        id: "windows-x86_64",
        name: "Windows",
        detail: "x86-64",
        kind: ArchiveKind::Zip,
    },
    Platform {
        id: "macos-arm64",
        name: "macOS",
        detail: "Apple silicon",
        kind: ArchiveKind::TarGz,
    },
];

fn page<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}
