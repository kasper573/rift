use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use engine::core::content::store::ContentStore;
use engine::core::platform::{
    FetchFailure, LocalFuture, Platform, ServerSocket, SignIn, SocketState, StartParams,
};
use engine::core::time::{LocalClock, Seconds, UnixMillis};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::store::WebStore;

const CANVAS: &str = "#glcanvas";
const STATUS: &str = "sync-status";
const RELOADED_FOR: &str = "rift.reloaded-for";

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

    fn engine(&self, plugins: PluginGroupBuilder) -> PluginGroupBuilder {
        plugins
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

    fn get(&self, url: String) -> LocalFuture<Result<Vec<u8>, FetchFailure>> {
        Box::pin(async move {
            let response = gloo_net::http::Request::get(&url)
                .send()
                .await
                .map_err(|error| FetchFailure::Transport(error.to_string()))?;
            refused(&response).await?;
            response
                .binary()
                .await
                .map_err(|error| FetchFailure::Transport(error.to_string()))
        })
    }

    fn post(
        &self,
        url: String,
        authorization: String,
        body: String,
    ) -> LocalFuture<Result<String, FetchFailure>> {
        Box::pin(async move {
            let response = gloo_net::http::Request::post(&url)
                .header("Authorization", &authorization)
                .header("Content-Type", "application/json")
                .body(body)
                .map_err(|error| FetchFailure::Transport(error.to_string()))?
                .send()
                .await
                .map_err(|error| FetchFailure::Transport(error.to_string()))?;
            refused(&response).await?;
            response
                .text()
                .await
                .map_err(|error| FetchFailure::Transport(error.to_string()))
        })
    }

    fn content_store(&self) -> Box<dyn ContentStore> {
        Box::new(WebStore)
    }

    fn announce(&self, status: &str) {
        if let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(STATUS))
        {
            element.set_text_content(Some(status));
        }
    }

    fn relaunch(&self, build: &str) -> Result<(), String> {
        let reloading = web_sys::window().is_some_and(|window| {
            window
                .session_storage()
                .ok()
                .flatten()
                .is_some_and(|session| {
                    session.get_item(RELOADED_FOR).ok().flatten().as_deref() != Some(build)
                        && session.set_item(RELOADED_FOR, build).is_ok()
                        && window.location().reload().is_ok()
                })
        });
        if reloading {
            Ok(())
        } else {
            Err("A new version of the game is live. Reload the page to play.".to_owned())
        }
    }

    fn local_clock(&self) -> LocalClock {
        let now = js_sys::Date::new_0();
        LocalClock {
            now: UnixMillis(now.get_time() as u64),
            utc_offset: Seconds(-now.get_timezone_offset() as f32 * 60.0),
        }
    }

    fn connect(&self, url: &str) -> Box<dyn ServerSocket> {
        Box::new(WsSocket::open(url))
    }
}

pub fn primary_window() -> Window {
    Window {
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
        sign_in: SignIn::Token(attribute("data-access-token")),
        game_server_url: attribute("data-game-server-url"),
        game_server_ws_url: attribute("data-game-server-ws-url"),
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

async fn refused(response: &gloo_net::http::Response) -> Result<(), FetchFailure> {
    if response.ok() {
        return Ok(());
    }
    Err(FetchFailure::Status {
        status: response.status(),
        body: response.text().await.unwrap_or_default(),
    })
}

struct WsSocket {
    ws: web_sys::WebSocket,
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
    state: Rc<Cell<SocketState>>,
    _handlers: [Closure<dyn FnMut()>; 3],
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
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

    fn state(&self) -> SocketState {
        self.state.get()
    }
}

fn closure_setting(state: &Rc<Cell<SocketState>>, to: SocketState) -> Closure<dyn FnMut()> {
    let state = state.clone();
    Closure::<dyn FnMut()>::new(move || state.set(to))
}
