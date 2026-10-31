use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bevy::prelude::*;

use crate::core::content::store::ContentStore;

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
    /// Signs in on the identity provider's own pages, in the system browser.
    Browser(Arc<oidc::BrowserSignIn>),
}

/// Resolves to the player's access token once they're through `page` in the system browser.
/// Dropping it gives up.
pub fn sign_in_in_browser(
    sign_in: &oidc::BrowserSignIn,
    page: oidc::AuthorizationPage,
) -> impl Future<Output = Result<String, String>> + Send + use<> {
    let pending = sign_in.begin(page, |url| {
        opener::open_browser(url).map_err(|error| format!("Couldn't open your browser: {error}"))
    });
    async move {
        let pending = pending?;
        let _hang_up = pending.hang_up();
        blocking::unblock(move || pending.finish()).await
    }
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

/// HTTP as the client speaks it: TLS against the OS trust store, so a locally trusted CA works like
/// any public one, and a refused status is a reply to read, not a transport failure.
#[derive(Resource, Clone)]
pub struct Http(pub ureq::Agent);

impl Default for Http {
    fn default() -> Http {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Http(ureq::Agent::new_with_config(config))
    }
}

impl Http {
    pub fn get(
        &self,
        url: String,
    ) -> impl Future<Output = Result<Vec<u8>, FetchFailure>> + Send + use<> {
        let agent = self.0.clone();
        blocking::unblock(move || {
            let mut response = agent
                .get(&url)
                .call()
                .map_err(|error| FetchFailure::Transport(error.to_string()))?;
            refused(&mut response)?;
            response
                .body_mut()
                .with_config()
                .limit(u64::MAX)
                .read_to_vec()
                .map_err(|error| FetchFailure::Transport(error.to_string()))
        })
    }

    pub fn post(
        &self,
        url: String,
        authorization: String,
        body: String,
    ) -> impl Future<Output = Result<String, FetchFailure>> + Send + use<> {
        let agent = self.0.clone();
        blocking::unblock(move || {
            let mut response = agent
                .post(&url)
                .header("Authorization", &authorization)
                .header("Content-Type", "application/json")
                .send(body)
                .map_err(|error| FetchFailure::Transport(error.to_string()))?;
            refused(&mut response)?;
            response
                .body_mut()
                .read_to_string()
                .map_err(|error| FetchFailure::Transport(error.to_string()))
        })
    }
}

/// The player's folder on this machine: their settings, and the cache of the game's content.
#[derive(Resource, Clone)]
pub struct Profile(pub PathBuf);

impl Profile {
    /// The player's settings as last saved, if ever.
    pub fn settings(&self) -> Option<String> {
        std::fs::read_to_string(self.0.join(SETTINGS)).ok()
    }

    pub fn save_settings(&self, text: &str) {
        let saved = std::fs::create_dir_all(&self.0)
            .and_then(|()| std::fs::write(self.0.join(SETTINGS), text));
        if let Err(error) = saved {
            eprintln!("settings not saved: {error}");
        }
    }

    pub fn content_store(&self) -> ContentStore {
        ContentStore::new(self.0.join(CACHE))
    }
}

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

const SETTINGS: &str = "settings.json";
const CACHE: &str = "cache";

fn refused(response: &mut ureq::http::Response<ureq::Body>) -> Result<(), FetchFailure> {
    if response.status().is_success() {
        return Ok(());
    }
    Err(FetchFailure::Status {
        status: response.status().as_u16(),
        body: response.body_mut().read_to_string().unwrap_or_default(),
    })
}
