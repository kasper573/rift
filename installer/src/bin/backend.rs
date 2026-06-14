use installer::metadata::files_from_urls;
use installer::service::{Release, router};
use serde::Deserialize;

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
    let _profiler = service::profiler(
        "rift-installer",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
    let release = Release {
        version: published.version,
        files: files_from_urls(&published.release_artifact_links),
    };
    service::serve("installer backend", port, router(release)).await;
}
