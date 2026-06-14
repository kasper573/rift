use std::sync::Arc;

use axum::routing::get;
use axum_prometheus::PrometheusMetricLayer;
use installer::metadata::files_from_urls;
use installer::service::{Release, router};
use pyroscope::backend::{BackendConfig, PprofConfig, pprof_backend};
use pyroscope::pyroscope::{PyroscopeAgent, PyroscopeAgentBuilder, PyroscopeAgentRunning};
use serde::Deserialize;

/// The `RIFT_INSTALLER_*` environment.
#[derive(Deserialize)]
struct Config {
    port: u16,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
}

/// The release the pipeline just published — its version and every artifact URL. Global `RIFT_*` vars
/// shared across apps, not installer-scoped, so the backend serves them without knowing where they're hosted.
#[derive(Deserialize)]
struct Published {
    version: String,
    release_artifact_links: Vec<String>,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_INSTALLER_")
        .from_env()
        .expect("RIFT_INSTALLER_* environment");
    let published: Published = envy::prefixed("RIFT_")
        .from_env()
        .expect("RIFT_VERSION and RIFT_RELEASE_ARTIFACT_LINKS environment");
    let port = config.port;
    // Held for the process lifetime: dropping the agent stops the profiler.
    let _profiler = if config.pyroscope_enabled {
        Some(start_profiler("rift-installer", config.pyroscope_sample_hz))
    } else {
        None
    };
    let (track, prometheus) = PrometheusMetricLayer::pair();
    metrics_process::Collector::default().describe();
    let release = Release {
        version: published.version,
        files: files_from_urls(&published.release_artifact_links),
    };
    let app = router(release).layer(track).route(
        "/metrics",
        get(move || async move {
            metrics_process::Collector::default().collect();
            prometheus.render()
        }),
    );
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|error| panic!("could not bind 0.0.0.0:{port}: {error}"));
    println!("installer backend listening on 0.0.0.0:{port}");
    // Nothing here is stateful, so a stop request (docker stop, deploys) drains in-flight requests then exits.
    let stop = Arc::new(tokio::sync::Notify::new());
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.notify_one()).expect("install stop handler");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { stop.notified().await })
        .await
        .expect("serve");
}

/// Continuously samples this process at `sample_hz` and pushes profiles to the `PYROSCOPE_BASE`
/// server under the given application name, where they surface in Grafana's profiles drilldown.
/// The returned agent must be held for the process lifetime, as dropping it stops profiling.
fn start_profiler(application: &str, sample_hz: u32) -> PyroscopeAgent<PyroscopeAgentRunning> {
    #[derive(Deserialize)]
    struct Profiling {
        base: String,
    }
    let profiling: Profiling = envy::prefixed("PYROSCOPE_")
        .from_env()
        .expect("PYROSCOPE_* environment");
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
    .expect("start pyroscope agent")
}
