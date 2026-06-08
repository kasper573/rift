//! In-network smoke checks against the docker test stack: run via `cargo x smoke` after
//! `cargo x up test` (the public hostnames resolve to the reverse proxy on the compose network).

use std::time::Duration;

#[test]
fn the_website_serves_the_landing_page() {
    let page = get(&format!("https://{}/", domain()));
    assert!(page.contains("<nav>"), "the nav shell must render");
    assert!(
        page.contains("Sign in"),
        "anonymous visitors must be offered sign-in"
    );
}

#[test]
fn the_website_requires_sign_in_to_play() {
    let page = get(&format!("https://{}/play", domain()));
    assert!(
        page.contains("You need to sign in to play."),
        "anonymous /play must ask for sign-in"
    );
}

#[test]
fn the_game_client_assets_are_served() {
    get(&format!("https://{}/game.wasm", domain()));
    get(&format!("https://{}/mq_js_bundle.js", domain()));
}

#[test]
fn the_game_server_is_healthy_through_the_proxy() {
    let health = get(&format!("https://game-server.{}/health", domain()));
    assert!(health.contains("ok"), "health endpoint must report ok");
}

#[test]
fn the_game_server_exposes_metrics_inside_the_stack() {
    let target = required_env("RIFT_GAME_SERVER_PROXY_HOST");
    let metrics = get(&format!("http://{target}/metrics"));
    assert!(
        metrics.contains("rift_ticks_total"),
        "metrics must include the tick counter"
    );
}

#[test]
fn the_keycloak_realm_is_up() {
    let realm = required_env("RIFT_AUTH__AUDIENCE");
    let config = get(&format!(
        "https://auth.{}/realms/{realm}/.well-known/openid-configuration",
        domain()
    ));
    assert!(
        config.contains("authorization_endpoint"),
        "the realm must serve its openid configuration"
    );
}

#[test]
fn grafana_is_up() {
    get(&format!("https://grafana.{}/login", domain()));
}

fn domain() -> String {
    required_env("RIFT_DOMAIN")
}

fn required_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("{name} must be set — run the smoke suite via `cargo x smoke`"))
}

fn get(url: &str) -> String {
    String::from_utf8_lossy(&get_bytes(url)).into_owned()
}

// The stack's reverse proxy uses a local throwaway CA, so verification is off.
fn get_bytes(url: &str) -> Vec<u8> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        )
        .build()
        .into();
    let mut last = String::new();
    for _ in 0..30 {
        match agent.get(url).call() {
            Ok(mut response) => {
                return response
                    .body_mut()
                    .read_to_vec()
                    .unwrap_or_else(|error| panic!("read {url}: {error}"));
            }
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    panic!("{url} never came up: {last}");
}
