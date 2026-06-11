//! The HTTP plumbing for sign-in and session minting: a shared `ureq` agent, and an adapter that
//! drives openidconnect's sync flow through it.

use std::time::Duration;

/// Verifies TLS against the OS trust store (not rustls' baked-in roots), so the dev proxy's
/// Caddy CA — trusted once per machine, see the README — works like any public CA.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

pub fn oidc_client()
-> impl Fn(::http::Request<Vec<u8>>) -> Result<::http::Response<Vec<u8>>, ureq::Error> {
    let agent = agent();
    move |request| {
        let response = agent.run(request)?;
        let (parts, mut body) = response.into_parts();
        let bytes = body.read_to_vec()?;
        Ok(::http::Response::from_parts(parts, bytes))
    }
}
