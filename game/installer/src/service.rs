use std::fmt;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::metadata::{Metadata, select_files};

/// The release's full file list before per-platform filtering. The backend's knowledge of where releases
/// come from lives behind this trait, so the router can be exercised against a fake source with no network.
pub trait ReleaseSource: Send + Sync {
    fn latest(&self) -> Result<Release, SourceError>;
}

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub files: Vec<crate::metadata::FileEntry>,
}

#[derive(Debug)]
pub struct SourceError(pub String);

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SourceError {}

/// A desktop installer, not a browser, is the only client, so there is no CORS, cookies, or HTML.
pub fn router(source: Arc<dyn ReleaseSource>) -> Router {
    Router::new()
        .route("/", get(manifest))
        .route("/health", get(health))
        .with_state(source)
}

#[derive(Deserialize)]
struct Platform {
    os: String,
    arch: String,
}

async fn manifest(
    State(source): State<Arc<dyn ReleaseSource>>,
    Query(platform): Query<Platform>,
) -> Response {
    // `latest` is blocking (a sync HTTP client), so it must not run on the async runtime's threads.
    let release = match tokio::task::spawn_blocking(move || source.latest()).await {
        Ok(Ok(release)) => release,
        Ok(Err(error)) => return (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let files = select_files(&release.files, &platform.os, &platform.arch);
    if files.is_empty() {
        return (StatusCode::NOT_FOUND, "no release files for this platform").into_response();
    }
    Json(Metadata {
        version: release.version,
        files,
    })
    .into_response()
}

async fn health() -> &'static str {
    "ok"
}
