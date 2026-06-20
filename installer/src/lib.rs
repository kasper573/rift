use std::time::Duration;

pub mod archive;
pub mod download;
pub mod metadata;
pub mod version;

#[cfg(feature = "backend")]
pub mod service;

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
