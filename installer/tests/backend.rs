#![cfg(feature = "backend")]

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use installer::metadata::{Metadata, files_from_urls};
use installer::service::{Release, router};
use tower::ServiceExt;

fn release() -> Release {
    let per_platform = ["linux-x86_64", "windows-x86_64"]
        .into_iter()
        .map(|platform| {
            let ext = if platform.starts_with("windows") {
                "zip"
            } else {
                "tar.gz"
            };
            let urls = vec![
                format!("https://example.test/rift-installer-{platform}.{ext}"),
                format!("https://example.test/rift-{platform}.{ext}"),
            ];
            (platform.to_owned(), files_from_urls(&urls))
        })
        .collect::<HashMap<_, _>>();
    Release {
        version: "0.110".to_owned(),
        per_platform,
        shared: files_from_urls(&["https://example.test/rift-assets.zip".to_owned()]),
    }
}

async fn get(uri: &str) -> (StatusCode, Vec<u8>) {
    let response = router(release())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn serves_the_platform_manifest_with_shared_assets() {
    let (status, body) = get("/?platform=linux-x86_64").await;
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
async fn another_platform_serves_only_its_own_binaries() {
    let (status, body) = get("/?platform=windows-x86_64").await;
    assert_eq!(status, StatusCode::OK);
    let manifest: Metadata = serde_json::from_slice(&body).unwrap();
    let names: Vec<_> = manifest
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "rift-installer-windows-x86_64.zip",
            "rift-windows-x86_64.zip",
            "rift-assets.zip",
        ]
    );
}

#[tokio::test]
async fn an_unsupported_platform_is_not_found() {
    let (status, _) = get("/?platform=plan9-sparc").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_without_a_platform_is_rejected() {
    let (status, _) = get("/").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
