//! Shared by the two installer binaries — `rift-installer` (the GUI frontend) and `installer-backend`
//! (the manifest service) — which never talk in code, only through the [`metadata`] manifest, so a
//! release's files can come from anywhere; the backend is handed their URLs and never knows the host.

use std::time::Duration;

pub mod archive;
pub mod download;
pub mod metadata;
pub mod version;

#[cfg(feature = "backend")]
pub mod service;

/// Verifies TLS against the OS trust store, so a privately trusted CA (a release mirror behind the dev
/// proxy) works like any public root. `global_timeout` bounds a whole request.
pub fn http_agent(global_timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(global_timeout))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build();
    ureq::Agent::new_with_config(config)
}
