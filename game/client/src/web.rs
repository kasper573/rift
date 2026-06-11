//! The HTTP plumbing for sign-in and session minting: a shared `ureq` agent that optionally trusts
//! an extra root CA (the dev reverse proxy's local Caddy CA, from `RIFT_CLIENT_EXTRA_CA`), and an
//! adapter that drives openidconnect's sync flow through that same agent.

use std::sync::Arc;
use std::time::Duration;

pub fn agent(extra_ca: Option<&[u8]>) -> ureq::Agent {
    let mut tls = ureq::tls::TlsConfig::builder();
    if let Some(pem) = extra_ca {
        let cert = ureq::tls::Certificate::from_pem(pem).expect("extra CA is valid PEM");
        tls = tls.root_certs(ureq::tls::RootCerts::Specific(Arc::new(vec![cert])));
    }
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .tls_config(tls.build())
        .build()
        .into()
}

/// openidconnect's sync HTTP client, backed by [`agent`] so discovery and token exchange honour
/// the extra CA.
pub fn oidc_client(
    extra_ca: Option<&[u8]>,
) -> impl Fn(::http::Request<Vec<u8>>) -> Result<::http::Response<Vec<u8>>, ureq::Error> {
    let agent = agent(extra_ca);
    move |request| {
        let response = agent.run(request)?;
        let (parts, mut body) = response.into_parts();
        let bytes = body.read_to_vec()?;
        Ok(::http::Response::from_parts(parts, bytes))
    }
}
