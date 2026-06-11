//! Browser automation for the e2e suite, driven by the headless_chrome crate: each stage owns one
//! Chrome with a throwaway profile.

use std::ffi::OsStr;
use std::sync::Arc;
use std::time::Duration;

use headless_chrome::LaunchOptions;
use headless_chrome::browser::tab::Tab;
use headless_chrome::protocol::cdp::Runtime;
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
        tab.navigate_to(url).expect("navigate");
        Page { tab }
    }
}

pub struct Page {
    tab: Arc<Tab>,
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
