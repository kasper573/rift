use std::sync::Arc;

use axum::Router;
use axum::ServiceExt;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum_prometheus::PrometheusMetricLayerBuilder;
use axum_prometheus::metrics_exporter_prometheus::PrometheusBuilder;
use pyroscope::backend::{BackendConfig, PprofConfig, pprof_backend};
use pyroscope::pyroscope::{PyroscopeAgent, PyroscopeAgentBuilder, PyroscopeAgentRunning};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tower_http::normalize_path::NormalizePath;

pub use axum_prometheus::metrics_exporter_prometheus::PrometheusHandle;
pub use tikv_jemallocator::Jemalloc;

#[macro_export]
macro_rules! heap_profiling {
    () => {
        #[global_allocator]
        static GLOBAL_ALLOCATOR: $crate::Jemalloc = $crate::Jemalloc;

        #[allow(non_upper_case_globals)]
        #[unsafe(export_name = "malloc_conf")]
        pub static MALLOC_CONF: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";
    };
}

pub async fn serve(name: &str, port: u16, router: Router) {
    let app = track(router, recorder());
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

pub fn recorder() -> PrometheusHandle {
    metrics_process::Collector::default().describe();
    PrometheusBuilder::new()
        .set_buckets(&[
            0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
        ])
        .expect("non-empty histogram buckets")
        .install_recorder()
        .expect("prometheus recorder installs")
}

pub fn track(router: Router, handle: PrometheusHandle) -> Router {
    let (layer, handle) = PrometheusMetricLayerBuilder::new()
        .with_metrics_from_fn(|| handle)
        .build_pair();
    router
        .layer(layer)
        .route(
            "/metrics",
            get(move || async move {
                metrics_process::Collector::default().collect();
                handle.render()
            }),
        )
        .route("/debug/pprof/heap", get(heap_profile))
}

async fn heap_profile() -> impl IntoResponse {
    let Some(controller) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return (StatusCode::NOT_FOUND, "heap profiling is not enabled").into_response();
    };
    match controller.lock().await.dump_pprof() {
        Ok(pprof) => pprof.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
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
