use std::time::Duration;

/// The game server's HTTP base for `/session`; the test stack's reverse proxy by default, or
/// `RIFT_E2E_GAME_SERVER` (e.g. `http://127.0.0.1:9998` against a locally run server).
pub fn base() -> String {
    std::env::var("RIFT_E2E_GAME_SERVER")
        .unwrap_or_else(|_| "https://game-server.rift.localhost".to_owned())
}

/// Mints a serialized `ConnectToken` for `authorization` (`Bearer <jwt>` or `Bypass <name>`).
pub fn mint(authorization: &str) -> Vec<u8> {
    // The test stack's reverse proxy uses a local throwaway CA, so verification is off.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        )
        .build()
        .into();
    let mut response = agent
        .post(format!("{}/session", base()))
        .header("Authorization", authorization)
        .send_empty()
        .expect("mint session token");
    assert!(
        response.status().is_success(),
        "session mint failed: {}",
        response.status()
    );
    response.body_mut().read_to_vec().expect("token bytes")
}
