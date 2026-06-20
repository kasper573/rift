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
    let stop = Arc::new(Notify::new());
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.notify_one()).expect("install stop handler");
    let app = NormalizePath::trim_trailing_slash(app);
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .with_graceful_shutdown(async move { stop.notified().await })
        .await
        .expect("serve");
}

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
