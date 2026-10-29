pub use std::alloc::System as ServiceAllocator;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::CpuProfileQuery;

pub async fn heap_profile() -> Response {
    (
        StatusCode::NOT_FOUND,
        "heap profiling runs only on Linux and macOS",
    )
        .into_response()
}

pub async fn cpu_profile(Query(_): Query<CpuProfileQuery>) -> Response {
    (
        StatusCode::NOT_FOUND,
        "CPU profiling runs only on Linux and macOS",
    )
        .into_response()
}
