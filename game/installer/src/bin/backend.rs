use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use installer::metadata::FileEntry;
use installer::service::{Release, ReleaseSource, SourceError, router};
use serde::Deserialize;

/// The `RIFT_INSTALLER_*` environment.
#[derive(Deserialize)]
struct Config {
    port: u16,
    github_repo: String,
}

#[tokio::main]
async fn main() {
    let config: Config = envy::prefixed("RIFT_INSTALLER_")
        .from_env()
        .expect("RIFT_INSTALLER_* environment");
    let port = config.port;
    let app = router(Arc::new(GithubSource::new(config.github_repo)));
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

/// Caches the parsed release briefly so a burst of installers costs one upstream call and stays under
/// GitHub's unauthenticated rate limit.
struct GithubSource {
    url: String,
    agent: ureq::Agent,
    cache: Mutex<Option<Cached>>,
}

struct Cached {
    release: Release,
    at: Instant,
}

impl GithubSource {
    const TTL: Duration = Duration::from_secs(60);

    fn new(repo: String) -> Self {
        Self {
            url: format!("https://api.github.com/repos/{repo}/releases/latest"),
            agent: installer::http_agent(Duration::from_secs(30)),
            cache: Mutex::new(None),
        }
    }

    fn fetch(&self) -> Result<Release, SourceError> {
        let body: GithubRelease = self
            .agent
            .get(&self.url)
            .header("User-Agent", "rift-installer")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| SourceError(format!("github request failed: {error}")))?
            .body_mut()
            .read_json()
            .map_err(|error| SourceError(format!("github response invalid: {error}")))?;
        Ok(Release {
            version: body.tag_name,
            files: body
                .assets
                .into_iter()
                .map(|asset| FileEntry {
                    name: asset.name,
                    url: asset.browser_download_url,
                })
                .collect(),
        })
    }
}

impl ReleaseSource for GithubSource {
    fn latest(&self) -> Result<Release, SourceError> {
        let mut cache = self.cache.lock().expect("cache lock");
        if let Some(cached) = cache.as_ref()
            && cached.at.elapsed() < Self::TTL
        {
            return Ok(cached.release.clone());
        }
        let release = self.fetch()?;
        *cache = Some(Cached {
            release: release.clone(),
            at: Instant::now(),
        });
        Ok(release)
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}
