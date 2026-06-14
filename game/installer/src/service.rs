use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::metadata::{FileEntry, Metadata, select_files};

/// The release's full file list before per-platform filtering. The pipeline owns where releases are
/// hosted and hands the backend their URLs, so the service itself knows nothing about the host.
pub struct Release {
    pub version: String,
    pub files: Vec<FileEntry>,
}

/// A desktop installer, not a browser, is the only client, so there is no CORS, cookies, or HTML.
pub fn router(release: Release) -> Router {
    Router::new()
        .route("/", get(manifest))
        .route("/health", get(health))
        .with_state(Arc::new(release))
}

#[derive(Deserialize)]
struct Platform {
    os: String,
    arch: String,
}

async fn manifest(
    State(release): State<Arc<Release>>,
    Query(platform): Query<Platform>,
) -> Response {
    let files = select_files(&release.files, &platform.os, &platform.arch);
    if files.is_empty() {
        return (StatusCode::NOT_FOUND, "no release files for this platform").into_response();
    }
    Json(Metadata {
        version: release.version.clone(),
        files,
    })
    .into_response()
}

async fn health() -> &'static str {
    "ok"
}
