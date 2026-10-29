use std::net::SocketAddr;

use serde::de::DeserializeOwned;
use web_server::{DESKTOP_SIGN_IN_ENV_PREFIX, ENV_PREFIX, Site, WebServerConfig};

service::heap_profiling!();

#[tokio::main]
async fn main() {
    let config: WebServerConfig = settings(ENV_PREFIX);
    let at = SocketAddr::new(config.bind, config.port);
    let game = web_server::published_game(&config.game_server_internal_url).await;
    println!("serving the site of {}", game.name);
    let site = Site {
        config,
        sign_in: settings(oidc::ENV_PREFIX),
        desktop_sign_in: settings(DESKTOP_SIGN_IN_ENV_PREFIX),
        game,
    };
    service::serve("web-server", at, web_server::router(site).await).await;
}

fn settings<T: DeserializeOwned>(prefix: &str) -> T {
    envy::prefixed(prefix)
        .from_env()
        .unwrap_or_else(|error| panic!("{prefix}* environment: {error}"))
}
