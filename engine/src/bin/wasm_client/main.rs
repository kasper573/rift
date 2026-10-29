mod platform;
mod store;

use std::sync::Arc;

use engine::systems::client_app;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::platform::{WebPlatform, primary_window, read_start_params};

// Edition 2024 spells every extern block unsafe; wasm-bindgen still exposes its imports as safe fns.
#[wasm_bindgen(module = "/src/bin/wasm_client/audio-unlock.js")]
unsafe extern "C" {
    fn audio_unlock();
}

// wasm-bindgen runs main during the page's `init()`; on native targets the browser imports panic.
fn main() {
    console_error_panic_hook::set_once();
    audio_unlock();
    let params = read_start_params();
    client_app(Arc::new(WebPlatform), params, primary_window()).run();
}
