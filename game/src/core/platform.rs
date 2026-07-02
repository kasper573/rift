use std::future::Future;
use std::pin::Pin;

use bevy::prelude::*;

#[derive(Resource)]
pub struct StartParams {
    pub access_token: Option<String>,
    pub game_server_url: String,
    pub game_server_ws_url: String,
}

/// The client's live byte stream to the server (a browser WebSocket in the web client), carrying
/// opaque renet packets with no netcode layer — so frames are buffered and drained each tick.
pub trait ServerSocket: 'static {
    fn recv(&self) -> Option<Vec<u8>>;
    fn send(&self, packet: &[u8]);
    fn is_open(&self) -> bool;
    fn is_closed(&self) -> bool;
}

/// Host capabilities the client needs that bevy doesn't provide.
/// Each target implements it in its binary and installs it as the [`ClientPlatform`] resource.
pub trait Platform: Send + Sync + 'static {
    fn load(&self, key: &str) -> Option<String>;
    fn save(&self, key: &str, value: &str);
    fn sync_window(&self, window: &mut Window);
    fn fetch(
        &self,
        url: String,
        authorization: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>>>>;
    fn connect(&self, url: &str) -> Box<dyn ServerSocket>;
}

#[derive(Resource)]
pub struct ClientPlatform(pub Box<dyn Platform>);
