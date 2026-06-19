//! The runtime every rift HTTP service shares: continuous profiling, request + process metrics on
//! `/metrics`, and graceful shutdown. A service supplies its routes and config; this supplies the rest.

use std::sync::Arc;

use axum::Router;
use axum::ServiceExt;
use axum::extract::Request;
use axum::routing::get;
use axum_prometheus::PrometheusMetricLayer;
use pyroscope::backend::{BackendConfig, PprofConfig, pprof_backend};
use pyroscope::pyroscope::{PyroscopeAgent, PyroscopeAgentBuilder, PyroscopeAgentRunning};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tower_http::normalize_path::NormalizePath;

/// Serves `router` on `0.0.0.0:{port}`, wrapping it with Prometheus request metrics plus process
/// metrics on `/metrics`, and draining in-flight requests on a stop signal (docker stop, deploys).
/// `name` is only used in the startup log line. Returns when the server stops.
pub async fn serve(name: &str, port: u16, router: Router) {
    let (track, prometheus) = PrometheusMetricLayer::pair();
    metrics_process::Collector::default().describe();
    let app = router.layer(track).route(
        "/metrics",
        get(move || async move {
            metrics_process::Collector::default().collect();
            prometheus.render()
        }),
    );
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|error| panic!("could not bind 0.0.0.0:{port}: {error}"));
    println!("{name} listening on 0.0.0.0:{port}");
    // ctrlc handles the platform signals; a Notify bridges its callback into the async shutdown.
    let stop = Arc::new(Notify::new());
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.notify_one()).expect("install stop handler");
    // Rewrite `/foo/` to `/foo` before routing so trailing slashes resolve; axum matches paths
    // exactly, so this must wrap the whole router rather than sit behind it as a per-route layer.
    let app = NormalizePath::trim_trailing_slash(app);
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .with_graceful_shutdown(async move { stop.notified().await })
        .await
        .expect("serve");
}

/// Continuously samples this process at `sample_hz` and pushes profiles to `PYROSCOPE_BASE` under
/// `application`, where they surface in Grafana's profiles drilldown. Returns `None` when disabled;
/// the agent must be held for the process lifetime, as dropping it stops profiling.
pub fn profiler(
    application: &str,
    enabled: bool,
    sample_hz: u32,
) -> Option<PyroscopeAgent<PyroscopeAgentRunning>> {
    if !enabled {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Profiling {
        base: String,
    }
    let profiling: Profiling = envy::prefixed("PYROSCOPE_")
        .from_env()
        .expect("PYROSCOPE_* environment");
    Some(
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
        .expect("start pyroscope agent"),
    )
}
