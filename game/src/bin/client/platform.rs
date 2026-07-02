use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use bevy::prelude::*;
use game::core::platform::{Platform, ServerSocket, StartParams};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

const CANVAS: &str = "#glcanvas";

pub struct WebPlatform;

impl Platform for WebPlatform {
    fn load(&self, key: &str) -> Option<String> {
        storage()?.get_item(key).ok()?
    }

    fn save(&self, key: &str, value: &str) {
        if let Some(storage) = storage() {
            let _ = storage.set_item(key, value);
        }
    }

    fn sync_window(&self, window: &mut Window) {
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

    fn fetch(
        &self,
        url: String,
        authorization: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
        Box::pin(async move {
            let response = gloo_net::http::Request::post(&url)
                .header("Authorization", &authorization)
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.ok() {
                return Err(format!("session request failed: {}", response.status()));
            }
            response.text().await.map_err(|error| error.to_string())
        })
    }

    fn connect(&self, url: &str) -> Box<dyn ServerSocket> {
        Box::new(WsSocket::open(url))
    }
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

pub fn expose_global_fn(name: &str, hook: impl Fn(f32, f32) + 'static) {
    let hook = Closure::<dyn Fn(f32, f32)>::new(hook);
    js_sys::Reflect::set(&js_sys::global(), &JsValue::from_str(name), hook.as_ref())
        .unwrap_or_else(|_| panic!("expose {name} on the JS global"));
    hook.forget(); // hand the closure to JS for the page's lifetime
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

struct WsSocket {
    ws: web_sys::WebSocket,
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
    state: Rc<Cell<SocketState>>,
    _handlers: [Closure<dyn FnMut()>; 3],
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

#[derive(Clone, Copy, PartialEq)]
enum SocketState {
    Connecting,
    Open,
    Closed,
}

impl WsSocket {
    fn open(url: &str) -> WsSocket {
        let ws = web_sys::WebSocket::new(url).expect("open websocket");
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let incoming = Rc::new(RefCell::new(VecDeque::new()));
        let state = Rc::new(Cell::new(SocketState::Connecting));

        let on_open = closure_setting(&state, SocketState::Open);
        let on_close = closure_setting(&state, SocketState::Closed);
        let on_error = closure_setting(&state, SocketState::Closed);
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        let on_message = {
            let incoming = incoming.clone();
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                    incoming
                        .borrow_mut()
                        .push_back(js_sys::Uint8Array::new(&buffer).to_vec());
                }
            })
        };
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        WsSocket {
            ws,
            incoming,
            state,
            _handlers: [on_open, on_close, on_error],
            _on_message: on_message,
        }
    }
}

impl ServerSocket for WsSocket {
    fn recv(&self) -> Option<Vec<u8>> {
        self.incoming.borrow_mut().pop_front()
    }

    fn send(&self, packet: &[u8]) {
        if self.state.get() == SocketState::Open {
            let _ = self.ws.send_with_u8_array(packet);
        }
    }

    fn is_open(&self) -> bool {
        self.state.get() == SocketState::Open
    }

    fn is_closed(&self) -> bool {
        self.state.get() == SocketState::Closed
    }
}

fn closure_setting(state: &Rc<Cell<SocketState>>, to: SocketState) -> Closure<dyn FnMut()> {
    let state = state.clone();
    Closure::<dyn FnMut()>::new(move || state.set(to))
}
