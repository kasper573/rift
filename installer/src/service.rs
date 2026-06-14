use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::metadata::{FileEntry, Metadata};

pub struct Release {
    pub version: String,
    pub per_platform: HashMap<String, Vec<FileEntry>>,
    pub shared: Vec<FileEntry>,
}

pub fn router(release: Release) -> Router {
    Router::new()
        .route("/", get(manifest))
        .route("/health", get(health))
        .with_state(Arc::new(release))
}

#[derive(Deserialize)]
struct PlatformQuery {
    platform: String,
}

async fn manifest(
    State(release): State<Arc<Release>>,
    Query(query): Query<PlatformQuery>,
) -> Response {
    let Some(platform_files) = release.per_platform.get(&query.platform) else {
        return (StatusCode::NOT_FOUND, "no release files for this platform").into_response();
    };
    let files: Vec<FileEntry> = platform_files
        .iter()
        .chain(&release.shared)
        .cloned()
        .collect();
    Json(Metadata {
        version: release.version.clone(),
        files,
    })
    .into_response()
}

async fn health() -> &'static str {
    "ok"
}
