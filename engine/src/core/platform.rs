use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;

use crate::core::content::store::ContentStore;
use crate::core::time::LocalClock;

pub type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Resource)]
pub struct StartParams {
    pub sign_in: SignIn,
    pub game_server_url: String,
    pub game_server_ws_url: String,
}

#[derive(Clone)]
pub enum SignIn {
    /// Signed in before the client started, as this access token.
    Token(String),
    /// Signs in as any account name the player gives, as a server on fake auth takes it.
    AnyAccount,
    Provider(Arc<dyn IdentityProvider>),
}

/// Signs players in on the identity provider's own pages.
pub trait IdentityProvider: Send + Sync + 'static {
    /// Resolves to the player's access token once they're through the `page`. Dropping it gives up.
    fn sign_in(&self, page: ProviderPage) -> LocalFuture<Result<String, String>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderPage {
    SignIn,
    CreateAccount,
}

/// Carries opaque renet packets with no netcode layer, so frames are buffered and drained each tick.
pub trait ServerSocket: 'static {
    fn recv(&self) -> Option<Vec<u8>>;
    fn send(&self, packet: &[u8]);
    fn state(&self) -> SocketState;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketState {
    Connecting,
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchFailure {
    Status { status: u16, body: String },
    Transport(String),
}

impl std::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FetchFailure::Status { status, body } if body.is_empty() => write!(f, "HTTP {status}"),
            FetchFailure::Status { status, body } => write!(f, "HTTP {status}: {body}"),
            FetchFailure::Transport(reason) => f.write_str(reason),
        }
    }
}

pub trait Platform: Send + Sync + 'static {
    fn load(&self, key: &str) -> Option<String>;
    fn save(&self, key: &str, value: &str);
    fn sync_window(&self, window: &mut Window);
    /// Whether the client draws in a window of its own, whose screen mode and refresh are the
    /// player's to set, rather than in a canvas the page around it sizes.
    fn owns_window(&self) -> bool;
    /// The engine's default plugins as this platform runs them.
    fn engine(&self, plugins: PluginGroupBuilder) -> PluginGroupBuilder;
    fn get(&self, url: String) -> LocalFuture<Result<Vec<u8>, FetchFailure>>;
    fn post(
        &self,
        url: String,
        authorization: String,
        body: String,
    ) -> LocalFuture<Result<String, FetchFailure>>;
    fn content_store(&self) -> Box<dyn ContentStore>;
    fn connect(&self, url: &str) -> Box<dyn ServerSocket>;
    fn local_clock(&self) -> LocalClock;
    fn announce(&self, status: &str);
    /// Restarts the client on the server's `build`, or says what the player has to do instead.
    fn relaunch(&self, build: &str) -> Result<(), String>;
}

#[derive(Resource, Clone)]
pub struct ClientPlatform(pub Arc<dyn Platform>);

/// Raised once whoever started the process closes its stdin, so a child that outlives a crashed
/// parent stops instead of lingering. The process ends from its own loop on seeing it, never from
/// the watching thread: a thread exiting the process while the main thread is inside the GPU driver
/// crashes in the driver's teardown.
#[derive(Clone, Default)]
pub struct StdinClosed(Arc<AtomicBool>);

impl StdinClosed {
    pub fn watch() -> StdinClosed {
        let closed = StdinClosed::default();
        let raise = closed.clone();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::stdin().lock(), &mut sink);
            raise.0.store(true, Ordering::Relaxed);
        });
        closed
    }

    pub fn is_closed(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Ends the app once the process's stdin closes.
pub fn exit_when_stdin_closes(app: &mut App) {
    let closed = StdinClosed::watch();
    app.add_systems(Update, move |mut exit: MessageWriter<AppExit>| {
        if closed.is_closed() {
            exit.write(AppExit::Success);
        }
    });
}
