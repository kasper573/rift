//! Browser automation for the e2e suite, driven by the headless_chrome crate: each stage owns
//! one Chrome with a throwaway profile, and every page records the binary WebSocket frames it
//! receives so a mirror session can replay them.

use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::prelude::*;
use headless_chrome::LaunchOptions;
use headless_chrome::browser::tab::Tab;
use headless_chrome::browser::tab::point::Point;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::{Network, Runtime};
use serde_json::Value;

pub struct Browser(headless_chrome::Browser);

impl Browser {
    pub fn launch() -> Browser {
        let options = LaunchOptions::default_builder()
            .sandbox(false)
            .window_size(Some((1280, 900)))
            .idle_browser_timeout(Duration::from_secs(120))
            .args(vec![OsStr::new("--enable-unsafe-swiftshader")])
            .build()
            .expect("launch options");
        Browser(headless_chrome::Browser::new(options).expect("launch chrome"))
    }

    pub fn open(&self, url: &str) -> Page {
        let tab = self.0.new_tab().expect("new tab");
        // Capture network activity (notably WebSocket frames) from the very first request.
        tab.call_method(Network::Enable {
            max_total_buffer_size: None,
            max_resource_buffer_size: None,
            max_post_data_size: None,
            enable_durable_messages: None,
            report_direct_socket_traffic: None,
        })
        .expect("network domain");
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&frames);
        tab.add_event_listener(Arc::new(move |event: &Event| {
            // Binary frames (opcode 2) arrive base64-encoded; the game speaks binary only.
            if let Event::NetworkWebSocketFrameReceived(received) = event
                && received.params.response.opcode == 2.0
                && let Ok(bytes) = BASE64_STANDARD.decode(&received.params.response.payload_data)
            {
                sink.lock().expect("frame sink").push(bytes);
            }
        }))
        .expect("frame listener");
        tab.navigate_to(url).expect("navigate");
        Page { tab, frames }
    }
}

pub struct Page {
    tab: Arc<Tab>,
    frames: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Page {
    pub fn eval(&self, expression: &str) -> Value {
        match self.evaluate(expression) {
            Ok((value, None)) => value,
            Ok((_, Some(details))) => {
                panic!("evaluate failed: {details:?} (expression: {expression})")
            }
            Err(error) => panic!("evaluate failed: {error} (expression: {expression})"),
        }
    }

    pub fn try_eval(&self, expression: &str) -> Value {
        match self.evaluate(expression) {
            Ok((value, None)) => value,
            _ => Value::Null,
        }
    }

    pub fn click(&self, x: f64, y: f64) {
        self.tab.click_point(Point { x, y }).expect("click");
    }

    pub fn received_frames(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.frames.lock().expect("frame sink"))
    }

    fn evaluate(
        &self,
        expression: &str,
    ) -> Result<(Value, Option<Runtime::ExceptionDetails>), String> {
        let evaluated = self
            .tab
            .call_method(Runtime::Evaluate {
                expression: expression.to_owned(),
                return_by_value: Some(true),
                generate_preview: Some(false),
                silent: Some(false),
                await_promise: Some(true),
                include_command_line_api: Some(false),
                user_gesture: Some(false),
                object_group: None,
                context_id: None,
                throw_on_side_effect: None,
                timeout: None,
                disable_breaks: None,
                repl_mode: None,
                allow_unsafe_eval_blocked_by_csp: None,
                unique_context_id: None,
                serialization_options: None,
            })
            .map_err(|error| error.to_string())?;
        Ok((
            evaluated.result.value.unwrap_or(Value::Null),
            evaluated.exception_details,
        ))
    }
}

/// Phase tracing for debugging harness hangs: set `RIFT_E2E_TRACE=1`.
pub fn trace(message: &str) {
    if std::env::var_os("RIFT_E2E_TRACE").is_some() {
        eprintln!("[e2e] {message}");
    }
}
