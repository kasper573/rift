//! Just enough Chrome DevTools Protocol for the browser stage; sessions are multiplexed over
//! one WebSocket (`flatten: true`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct Browser {
    process: Child,
    pub cdp: Rc<RefCell<Cdp>>,
    _profile: tempdir::TempDir,
}

impl Browser {
    pub fn launch() -> Browser {
        let profile = tempdir::TempDir::new();
        let binary = std::env::var("CHROME").unwrap_or_else(|_| "google-chrome".to_owned());
        let mut process = Command::new(&binary)
            .args([
                "--headless=new",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--no-sandbox",
                "--disable-gpu",
                "--enable-unsafe-swiftshader",
                "--ignore-certificate-errors",
                "--window-size=1280,900",
            ])
            .arg(format!("--user-data-dir={}", profile.path.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("could not launch {binary}: {error}"));

        // Chrome prints the DevTools WebSocket endpoint on stderr.
        let stderr = process.stderr.take().expect("piped stderr");
        let mut lines = BufReader::new(stderr).lines();
        let deadline = Instant::now() + Duration::from_secs(20);
        let url = loop {
            assert!(Instant::now() < deadline, "chrome never announced DevTools");
            let Some(Ok(line)) = lines.next() else {
                panic!("chrome exited before announcing DevTools");
            };
            if let Some(rest) = line.strip_prefix("DevTools listening on ") {
                break rest.trim().to_owned();
            }
        };
        // Keep draining stderr so chrome never blocks on a full pipe.
        std::thread::spawn(move || for _line in lines {});

        trace(&format!("chrome devtools at {url}"));
        let (socket, _) = tungstenite::connect(&url).expect("connect to DevTools");
        trace("devtools connected");
        // A short timeout keeps reads responsive: pump() returns at the first quiet gap, and
        // call() polls for its response without long stalls.
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream
                .set_read_timeout(Some(Duration::from_millis(10)))
                .expect("set read timeout");
        }
        Browser {
            process,
            cdp: Rc::new(RefCell::new(Cdp {
                socket,
                next_id: 1,
                events: VecDeque::new(),
            })),
            _profile: profile,
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

pub struct Cdp {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    events: VecDeque<Value>,
}

/// Phase tracing for debugging harness hangs: set `RIFT_E2E_TRACE=1`.
pub fn trace(message: &str) {
    if std::env::var_os("RIFT_E2E_TRACE").is_some() {
        eprintln!("[e2e] {message}");
    }
}

impl Cdp {
    pub fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        trace(&format!("cdp -> {method} (id {id})"));
        self.socket
            .send(Message::text(message.to_string()))
            .expect("send CDP command");
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let Some(value) = self.read_one() else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    panic!("CDP {method} failed: {error}");
                }
                trace(&format!("cdp <- {method} (id {id})"));
                return value.get("result").cloned().unwrap_or(Value::Null);
            }
            self.events.push_back(value);
        }
        panic!("CDP {method} timed out");
    }

    // Bounded drain: a live game streams events continuously, so "read until quiet" would
    // never return.
    pub fn pump(&mut self) {
        for _ in 0..1024 {
            let Some(value) = self.read_one() else {
                break;
            };
            if value.get("method").and_then(Value::as_str) == Some("Network.webSocketFrameReceived")
            {
                self.events.push_back(value);
            }
        }
        self.events.retain(|value| {
            value.get("method").and_then(Value::as_str) == Some("Network.webSocketFrameReceived")
        });
    }

    pub fn take_frames(&mut self, session: &str) -> Vec<Value> {
        let mut taken = Vec::new();
        self.events.retain(|value| {
            if value.get("sessionId").and_then(Value::as_str) == Some(session) {
                taken.push(value.clone());
                false
            } else {
                true
            }
        });
        taken
    }

    fn read_one(&mut self) -> Option<Value> {
        match self.socket.read() {
            Ok(Message::Text(text)) => serde_json::from_str(&text).ok(),
            Ok(_) => None,
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                None
            }
            Err(error) => panic!("CDP socket failed: {error}"),
        }
    }
}

pub struct Page {
    pub cdp: Rc<RefCell<Cdp>>,
    pub session: String,
    target: String,
    context: String,
}

impl Page {
    pub fn open(cdp: &Rc<RefCell<Cdp>>, url: &str) -> Page {
        let mut guard = cdp.borrow_mut();
        let context =
            guard.call(None, "Target.createBrowserContext", json!({}))["browserContextId"]
                .as_str()
                .expect("context id")
                .to_owned();
        let target = guard.call(
            None,
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": context }),
        )["targetId"]
            .as_str()
            .expect("target id")
            .to_owned();
        let session = guard.call(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
        )["sessionId"]
            .as_str()
            .expect("session id")
            .to_owned();
        guard.call(Some(&session), "Page.enable", json!({}));
        guard.call(Some(&session), "Runtime.enable", json!({}));
        // Capture network activity (notably WebSocket frames) from the very first request.
        guard.call(Some(&session), "Network.enable", json!({}));
        guard.call(Some(&session), "Page.navigate", json!({ "url": url }));
        drop(guard);
        Page {
            cdp: Rc::clone(cdp),
            session,
            target,
            context,
        }
    }

    pub fn eval(&self, expression: &str) -> Value {
        let result = self.cdp.borrow_mut().call(
            Some(&self.session),
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        );
        if let Some(details) = result.get("exceptionDetails") {
            panic!("evaluate failed: {details} (expression: {expression})");
        }
        result["result"]["value"].clone()
    }

    pub fn try_eval(&self, expression: &str) -> Value {
        let result = self.cdp.borrow_mut().call(
            Some(&self.session),
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        );
        if result.get("exceptionDetails").is_some() {
            return Value::Null;
        }
        result["result"]["value"].clone()
    }

    pub fn click(&self, x: f64, y: f64) {
        let mut cdp = self.cdp.borrow_mut();
        for kind in ["mousePressed", "mouseReleased"] {
            cdp.call(
                Some(&self.session),
                "Input.dispatchMouseEvent",
                json!({
                    "type": kind,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "clickCount": 1,
                }),
            );
        }
    }

    pub fn press_key(&self, code: &str, key: &str) {
        let mut cdp = self.cdp.borrow_mut();
        for kind in ["keyDown", "keyUp"] {
            cdp.call(
                Some(&self.session),
                "Input.dispatchKeyEvent",
                json!({ "type": kind, "code": code, "key": key }),
            );
        }
    }

    pub fn received_frames(&self) -> Vec<Vec<u8>> {
        let mut cdp = self.cdp.borrow_mut();
        cdp.pump();
        cdp.take_frames(&self.session)
            .iter()
            .filter_map(|event| {
                let response = &event["params"]["response"];
                let payload = response["payloadData"].as_str()?;
                // Binary frames (opcode 2) arrive base64-encoded; the game speaks binary only.
                (response["opcode"].as_u64() == Some(2)).then(|| base64_decode(payload))
            })
            .collect()
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let mut cdp = self.cdp.borrow_mut();
        cdp.call(
            None,
            "Target.closeTarget",
            json!({ "targetId": self.target }),
        );
        cdp.call(
            None,
            "Target.disposeBrowserContext",
            json!({ "browserContextId": self.context }),
        );
    }
}

pub fn base64_decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let value_of = |byte: u8| TABLE.iter().position(|&t| t == byte).map(|i| i as u32);
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for &byte in text.as_bytes() {
        let Some(value) = value_of(byte) else {
            continue; // '=' padding and whitespace
        };
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    out
}

mod tempdir {
    pub struct TempDir {
        pub path: std::path::PathBuf,
    }
    impl TempDir {
        pub fn new() -> TempDir {
            let path = std::env::terift_dir().join(format!(
                "rift-e2e-chrome-{}-{:x}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("epoch")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).expect("create chrome profile dir");
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
