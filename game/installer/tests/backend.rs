#![cfg(feature = "backend")]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use installer::metadata::{FileEntry, Metadata};
use installer::service::{Release, ReleaseSource, SourceError, router};
use tower::ServiceExt;

struct FakeSource;

impl ReleaseSource for FakeSource {
    fn latest(&self) -> Result<Release, SourceError> {
        Ok(Release {
            version: "0.110".to_owned(),
            files: ["linux-x86_64", "windows-x86_64"]
                .iter()
                .flat_map(|platform| {
                    [
                        file(&format!("rift-installer-{platform}.tar.gz")),
                        file(&format!("rift-{platform}.tar.gz")),
                    ]
                })
                .chain([file("rift-assets.zip")])
                .collect(),
        })
    }
}

fn file(name: &str) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        url: format!("https://example.test/{name}"),
    }
}

async fn get(uri: &str) -> (StatusCode, Vec<u8>) {
    let response = router(Arc::new(FakeSource))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn serves_the_platform_filtered_manifest() {
    let (status, body) = get("/?os=linux&arch=x86_64").await;
    assert_eq!(status, StatusCode::OK);
    let manifest: Metadata = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest.version, "0.110");
    let names: Vec<_> = manifest
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "rift-installer-linux-x86_64.tar.gz",
            "rift-linux-x86_64.tar.gz",
            "rift-assets.zip",
        ]
    );
}

#[tokio::test]
async fn an_unsupported_platform_is_not_found() {
    let (status, _) = get("/?os=plan9&arch=sparc").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_without_platform_params_is_rejected() {
    let (status, _) = get("/").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
