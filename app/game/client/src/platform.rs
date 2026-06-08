pub use imp::*;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use macroquad::input::{KeyCode, is_key_pressed};
    use world::MmoClient;

    pub async fn open_session() -> Option<MmoClient> {
        let args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|arg| arg != "--spectate")
            .collect();
        let address = args
            .first()
            .cloned()
            .unwrap_or_else(|| world::DEFAULT_ADDRESS.to_owned());
        let token = args.get(1).cloned().unwrap_or_default();
        match MmoClient::connect(&address, &token) {
            Ok(client) => Some(client),
            Err(error) => {
                eprintln!("could not connect to {address}: {error}");
                None
            }
        }
    }

    pub fn spectate_mode() -> bool {
        std::env::args().any(|arg| arg == "--spectate")
    }

    pub fn exit_requested() -> bool {
        is_key_pressed(KeyCode::Escape)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    use world::{LinkStatus, MmoClient, Transport};

    static SPECTATE: AtomicBool = AtomicBool::new(false);

    // The contract with js/rift_ws.js (appended to mq_js_bundle.js at staging): state is
    // 0 connecting / 1 open / 2 closed, next returns -1 when empty. Values stay i32 — JS plugin
    // functions return Numbers, and wasm i64 imports take BigInt.
    unsafe extern "C" {
        fn rift_ws_open(pointer: *const u8, length: usize);
        fn rift_ws_state() -> i32;
        fn rift_ws_send(pointer: *const u8, length: usize);
        fn rift_ws_next() -> i32;
        fn rift_ws_read(pointer: *mut u8);
    }

    /// Fetched relative to the page, so the play and spectate pages each answer with their own
    /// parameters: a two-line body (ws url, spectate flag) produced by the game page handlers
    /// in app/website.
    pub async fn open_session() -> Option<MmoClient> {
        let body = macroquad::file::load_file("?config").await.ok()?;
        let body = String::from_utf8_lossy(&body);
        let mut lines = body.lines();
        let url = lines.next().unwrap_or_default();
        SPECTATE.store(lines.next() == Some("1"), Ordering::Relaxed);
        unsafe { rift_ws_open(url.as_ptr(), url.len()) };
        Some(MmoClient::with_transport(Box::new(WsBridge {
            scratch: Vec::new(),
        })))
    }

    pub fn spectate_mode() -> bool {
        SPECTATE.load(Ordering::Relaxed)
    }

    pub fn exit_requested() -> bool {
        false
    }

    struct WsBridge {
        scratch: Vec<u8>,
    }

    impl Transport for WsBridge {
        fn send(&mut self, packet: &[u8]) {
            unsafe { rift_ws_send(packet.as_ptr(), packet.len()) };
        }

        fn poll(&mut self, sink: &mut dyn FnMut(&[u8])) {
            loop {
                let length = unsafe { rift_ws_next() };
                if length < 0 {
                    return;
                }
                self.scratch.resize(length as usize, 0);
                unsafe { rift_ws_read(self.scratch.as_mut_ptr()) };
                sink(&self.scratch);
            }
        }

        fn status(&self) -> LinkStatus {
            match unsafe { rift_ws_state() } {
                0 => LinkStatus::Connecting,
                1 => LinkStatus::Open,
                _ => LinkStatus::Closed,
            }
        }
    }
}
