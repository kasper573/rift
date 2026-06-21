use bevy::prelude::Resource;

/// Parameters the website injects into the `#glcanvas` element: the access token (absent only if the
/// page somehow loaded signed-out), the game server's HTTPS API origin (for the session request), and
/// its WebSocket origin (for the netcode connection).
#[derive(Resource)]
pub struct StartParams {
    pub access_token: Option<String>,
    pub game_server_url: String,
    pub game_server_ws_url: String,
}

pub fn read() -> StartParams {
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector("#glcanvas").ok().flatten())
        .expect("#glcanvas element");
    let attribute = |name: &str| {
        canvas
            .get_attribute(name)
            .unwrap_or_else(|| panic!("#glcanvas needs a {name} attribute"))
    };
    StartParams {
        access_token: canvas
            .get_attribute("data-access-token")
            .filter(|token| !token.is_empty()),
        game_server_url: attribute("data-game-server-url"),
        game_server_ws_url: attribute("data-game-server-ws-url"),
    }
}
