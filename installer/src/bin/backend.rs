use std::collections::HashMap;

use installer::metadata::files_from_urls;
use installer::service::{Release, router};
use serde::{Deserialize, Deserializer};

service::heap_profiling!();

#[derive(Deserialize)]
struct Config {
    port: u16,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
    #[serde(deserialize_with = "json_env")]
    per_platform_artifact_links: HashMap<String, Vec<String>>,
    shared_artifact_links: Vec<String>,
}

#[derive(Deserialize)]
struct Published {
    version: String,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_INSTALLER_")
        .from_env()
        .expect("RIFT_INSTALLER_* environment");
    let published: Published = envy::prefixed("RIFT_")
        .from_env()
        .expect("RIFT_VERSION environment");
    let port = config.port;
    let _profiler = service::profiler(
        "rift-installer",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
    let release = Release {
        version: published.version,
        per_platform: config
            .per_platform_artifact_links
            .iter()
            .map(|(platform, urls)| (platform.clone(), files_from_urls(urls)))
            .collect(),
        shared: files_from_urls(&config.shared_artifact_links),
    };
    service::serve("installer backend", port, router(release)).await;
}

fn json_env<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = String::deserialize(deserializer)?;
    serde_json::from_str(&raw).map_err(serde::de::Error::custom)
}
