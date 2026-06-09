// ci-probe 8
use std::time::{Duration, Instant};

use rift::TcpCluster;
use world::{TICK_HZ, features, spawn_zone, zones};

fn main() -> std::io::Result<()> {
    let address = std::env::var("RIFT_GAME_SERVER_PORT")
        .map(|port| {
            let hostname =
                std::env::var("RIFT_GAME_SERVER_HOSTNAME").unwrap_or_else(|_| "0.0.0.0".to_owned());
            format!("{hostname}:{port}")
        })
        .ok()
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_else(|| world::DEFAULT_ADDRESS.to_owned());

    let mut host = TcpCluster::bind(&address, &features(), &zones(), spawn_zone())?;
    if let Some(authenticator) = authenticator_from_env() {
        host.authenticate_with(authenticator);
    }
    println!("mmo server listening on {}", host.local_addr());

    let delta_time = 1.0 / TICK_HZ;
    let frame = Duration::from_secs_f32(delta_time);
    loop {
        let started = Instant::now();
        host.poll();
        host.tick(delta_time);
        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

/// Keycloak verification configured through `RIFT_GAME_SERVER_AUTH__*`; without an issuer the
/// server runs open (plain local development).
fn authenticator_from_env() -> Option<rift::Authenticator> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let issuer = var("RIFT_GAME_SERVER_AUTH__ISSUER")?;
    let audience = var("RIFT_GAME_SERVER_AUTH__AUDIENCE")?;
    let jwks_uri = var("RIFT_GAME_SERVER_AUTH__JWKS_URI")?;
    let allow_bypass = var("RIFT_GAME_SERVER_AUTH__ALLOW_BYPASS_USERS")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));

    let mut verifier = auth::Verifier::new(&issuer, &audience, &jwks_uri, allow_bypass);
    match verifier.warm() {
        Ok(()) => println!("auth enabled, issuer {issuer}"),
        Err(error) => println!("auth enabled, issuer {issuer} (jwks warm-up failed: {error})"),
    }
    Some(Box::new(move |token| {
        verifier.verify(token).map(|claims| {
            std::sync::Arc::new(world::Identity {
                id: claims.subject,
                name: claims.name,
                roles: claims.roles,
            }) as rift::Session
        })
    }))
}
