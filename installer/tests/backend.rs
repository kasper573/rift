#![cfg(feature = "backend")]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use installer::metadata::{Metadata, files_from_urls};
use installer::service::{Release, router};
use tower::ServiceExt;

fn release() -> Release {
    let urls: Vec<String> = ["linux-x86_64", "windows-x86_64"]
        .iter()
        .flat_map(|platform| {
            [
                format!("https://example.test/rift-installer-{platform}.tar.gz"),
                format!("https://example.test/rift-{platform}.tar.gz"),
            ]
        })
        .chain(["https://example.test/rift-assets.zip".to_owned()])
        .collect();
    Release {
        version: "0.110".to_owned(),
        files: files_from_urls(&urls),
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
