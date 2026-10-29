pub use tikv_jemallocator::Jemalloc as ServiceAllocator;

use std::time::Duration;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pprof::protos::Message;

use crate::CpuProfileQuery;

pub async fn heap_profile() -> Response {
    let Some(controller) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return (StatusCode::NOT_FOUND, "heap profiling is not enabled").into_response();
    };
    match controller.lock().await.dump_pprof() {
        Ok(pprof) => pprof.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub async fn cpu_profile(Query(query): Query<CpuProfileQuery>) -> Response {
    let sampled = tokio::task::spawn_blocking(move || sample(Duration::from_secs(query.seconds)))
        .await
        .expect("the sampling thread finishes");
    match sampled {
        Ok(pprof) => pprof.into_response(),
        Err(error) => (StatusCode::CONFLICT, error).into_response(),
    }
}

// Unwinding through these from the sampling signal can deadlock or read bad unwind tables.
const UNWIND_BLOCKLIST: [&str; 4] = ["libc", "libgcc", "pthread", "vdso"];

fn sample(lasting: Duration) -> Result<Vec<u8>, String> {
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(SAMPLE_HZ)
        .blocklist(&UNWIND_BLOCKLIST)
        .build()
        .map_err(|error| format!("cannot start sampling: {error}"))?;
    std::thread::sleep(lasting);
    let profile = guard
        .report()
        .build()
        .and_then(|report| report.pprof())
        .map_err(|error| format!("cannot build the profile: {error}"))?;
    Ok(profile.encode_to_vec())
}

const SAMPLE_HZ: i32 = 100;
