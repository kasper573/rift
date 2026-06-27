use std::time::Duration;

use bevy::prelude::*;
use renet2_netcode::{ClientSocket, WebSocketClient, WebSocketClientConfig};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;

#[derive(Resource)]
pub struct StartParams {
    pub access_token: Option<String>,
    pub game_server_url: String,
    pub game_server_ws_url: String,
}

const CANVAS: &str = "#glcanvas";

#[wasm_bindgen(module = "/audio-unlock.js")]
unsafe extern "C" {
    fn audio_unlock();
}

#[wasm_bindgen]
pub fn run() {
    set_panic_hook();
    audio_unlock();
    crate::boot();
}

fn set_panic_hook() {
    console_error_panic_hook::set_once();
}

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

pub fn now() -> Duration {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .expect("system clock")
}

pub fn client_socket(ws_url: &str) -> impl ClientSocket {
    WebSocketClient::new(WebSocketClientConfig {
        server_url: url::Url::parse(ws_url).expect("game server ws url"),
    })
    .expect("websocket client")
}
