//! The deploy-target adapter. The rest of the client is platform-agnostic Bevy; everything specific
//! to *how* it is shipped — here, a wasm app in a browser tab — is concentrated in this one module.
//! Retargeting (a native window, a different host) is a matter of swapping this module's body, not
//! touching the engine code, which talks only to the functions below.

use std::time::Duration;

use bevy::prelude::*;
use renet2_netcode::{ClientSocket, WebSocketClient, WebSocketClientConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;

/// Boot parameters the host hands the client: the player's access token (absent only if the page
/// somehow loaded signed-out), the game server's HTTPS API origin (for the session request) and its
/// WebSocket origin (for the netcode connection). On the web the host writes them onto the canvas.
#[derive(Resource)]
pub struct StartParams {
    pub access_token: Option<String>,
    pub game_server_url: String,
    pub game_server_ws_url: String,
}

const CANVAS: &str = "#glcanvas";

/// The entry point the host invokes (the page's loader calls this, exported as `run`, after `init()`).
#[wasm_bindgen]
pub fn run() {
    set_panic_hook();
    crate::boot();
}

/// Routes panics somewhere visible — the browser console, which the page also surfaces on a crash.
fn set_panic_hook() {
    console_error_panic_hook::set_once();
}

/// The primary window the app binds to: the host's existing canvas element.
pub fn primary_window() -> Window {
    Window {
        title: "rift mmo".to_owned(),
        canvas: Some(CANVAS.to_owned()),
        ..default()
    }
}

pub fn read_start_params() -> StartParams {
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(CANVAS).ok().flatten())
        .expect("canvas element");
    let attribute = |name: &str| {
        canvas
            .get_attribute(name)
            .unwrap_or_else(|| panic!("canvas needs a {name} attribute"))
    };
    StartParams {
        access_token: canvas
            .get_attribute("data-access-token")
            .filter(|token| !token.is_empty()),
        game_server_url: attribute("data-game-server-url"),
        game_server_ws_url: attribute("data-game-server-ws-url"),
    }
}

/// Keeps the window's resolution and scale factor matched to the display surface. bevy binds to the
/// host's existing canvas but never sizes its backing buffer, so without this it stays the 300x150
/// HTML default and the browser stretches it (blurry). bevy lays the UI and cameras out in logical
/// pixels but renders at physical pixels (logical x scale factor), so the backing must be physical-
/// pixel sized: a logical-sized backing on a high-DPI display is both blurry and too small, and bevy's
/// physical-pixel render then gets clipped to a corner, throwing the whole view off-centre. On the web
/// the scale factor is the device pixel ratio, so the backing is logical x DPR and the override matches.
pub fn sync_window(window: &mut Window) {
    let Some(web) = web_sys::window() else {
        return;
    };
    let Some(canvas) = web
        .document()
        .and_then(|document| document.query_selector(CANVAS).ok().flatten())
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        return;
    };
    let (logical_w, logical_h) = (canvas.client_width(), canvas.client_height());
    if logical_w <= 0 || logical_h <= 0 {
        return;
    }
    let dpr = web.device_pixel_ratio().max(1.0);
    let physical_w = (logical_w as f64 * dpr).round() as u32;
    let physical_h = (logical_h as f64 * dpr).round() as u32;
    if canvas.width() != physical_w {
        canvas.set_width(physical_w);
    }
    if canvas.height() != physical_h {
        canvas.set_height(physical_h);
    }
    if window.resolution.scale_factor() != dpr as f32 {
        window
            .resolution
            .set_scale_factor_override(Some(dpr as f32));
    }
    let (logical_w, logical_h) = (logical_w as f32, logical_h as f32);
    if window.resolution.width() != logical_w || window.resolution.height() != logical_h {
        window.resolution.set(logical_w, logical_h);
    }
}

/// Persistent key/value storage for user settings (the browser's local storage).
pub fn load(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

pub fn save(key: &str, value: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(key, value);
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// POSTs to `url` with the given authorization and returns the raw response body — the host's network
/// stack (the browser's fetch).
pub async fn fetch(url: &str, authorization: &str) -> Result<Vec<u8>, String> {
    let response = gloo_net::http::Request::post(url)
        .header("Authorization", authorization)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(format!("session request failed: {}", response.status()));
    }
    response.binary().await.map_err(|error| error.to_string())
}

/// Wall-clock time since the unix epoch, for the netcode handshake.
pub fn now() -> Duration {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .expect("system clock")
}

/// The netcode transport socket to the game server. Returns `impl ClientSocket` so the concrete
/// transport (here a browser WebSocket) stays inside the adapter — the netcode wiring stays generic.
pub fn client_socket(ws_url: &str) -> impl ClientSocket {
    WebSocketClient::new(WebSocketClientConfig {
        server_url: url::Url::parse(ws_url).expect("game server ws url"),
    })
    .expect("websocket client")
}
