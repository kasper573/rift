use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use bevy::app::PluginGroupBuilder;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::window::Window;
use engine::core::content::Content;
use engine::core::content::store::ContentStore;
use engine::core::platform::{
    FetchFailure, IdentityProvider, LocalFuture, Platform, ProviderPage, ServerSocket, SocketState,
};
use engine::core::time::{LocalClock, Seconds};
use engine::systems::probe::{self, ProbeChannel};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use crate::store::FileStore;

const SETTINGS: &str = "settings.json";
const CACHE: &str = "cache";

pub struct DesktopPlatform {
    profile: PathBuf,
    agent: ureq::Agent,
}

impl DesktopPlatform {
    pub fn new(profile: PathBuf, agent: ureq::Agent) -> DesktopPlatform {
        DesktopPlatform { profile, agent }
    }

    fn settings(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.profile.join(SETTINGS))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }
}

impl Platform for DesktopPlatform {
    fn load(&self, key: &str) -> Option<String> {
        self.settings().remove(key)
    }

    fn save(&self, key: &str, value: &str) {
        let mut settings = self.settings();
        settings.insert(key.to_owned(), value.to_owned());
        let text = serde_json::to_string_pretty(&settings).expect("settings serialize");
        if std::fs::create_dir_all(&self.profile).is_ok()
            && let Err(error) = std::fs::write(self.profile.join(SETTINGS), text)
        {
            eprintln!("settings not saved: {error}");
        }
    }

    fn sync_window(&self, _window: &mut Window) {}

    // Rendering a frame alongside the next update puts the interface on screen a frame later,
    // trailing the system cursor, and the client's frames are cheap enough to render in turn.
    fn engine(&self, plugins: PluginGroupBuilder) -> PluginGroupBuilder {
        plugins.disable::<PipelinedRenderingPlugin>()
    }

    fn get(&self, url: String) -> LocalFuture<Result<Vec<u8>, FetchFailure>> {
        let agent = self.agent.clone();
        Box::pin(blocking::unblock(move || {
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
        }))
    }

    fn post(
        &self,
        url: String,
        authorization: String,
        body: String,
    ) -> LocalFuture<Result<String, FetchFailure>> {
        let agent = self.agent.clone();
        Box::pin(blocking::unblock(move || {
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
        }))
    }

    fn content_store(&self) -> Box<dyn ContentStore> {
        Box::new(FileStore::new(self.profile.join(CACHE)))
    }

    fn connect(&self, url: &str) -> Box<dyn ServerSocket> {
        Box::new(ThreadSocket::open(url))
    }

    fn local_clock(&self) -> LocalClock {
        LocalClock {
            now: SystemTime::now().into(),
            utc_offset: Seconds(chrono::Local::now().offset().local_minus_utc() as f32),
        }
    }

    fn announce(&self, status: &str) {
        eprintln!("{status}");
    }

    fn relaunch(&self, build: &str) -> Result<(), String> {
        Err(format!(
            "The server runs build {build}. Start a desktop client of that build to play."
        ))
    }
}

// TLS against the OS trust store, so a locally trusted CA works like any public one; a refused status
// is a reply to read, not a transport failure.
/// Signs in through the provider's pages in the system browser.
pub struct BrowserIdentity(pub oidc::BrowserSignIn);

impl IdentityProvider for BrowserIdentity {
    fn sign_in(&self, page: ProviderPage) -> LocalFuture<Result<String, String>> {
        let page = match page {
            ProviderPage::SignIn => oidc::AuthorizationPage::SignIn,
            ProviderPage::CreateAccount => oidc::AuthorizationPage::SignUp,
        };
        let pending = self.0.begin(page, |url| {
            opener::open_browser(url)
                .map_err(|error| format!("Couldn't open your browser: {error}"))
        });
        Box::pin(async move {
            let pending = pending?;
            let _hang_up = pending.hang_up();
            blocking::unblock(move || pending.finish()).await
        })
    }
}

pub fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build();
    ureq::Agent::new_with_config(config)
}

// The driver asks with a line and waits for its answer: a snapshot of a frame after it, or what the
// game says, answered on the socket's own thread once the game is loaded.
pub struct SocketProbe {
    asked: Receiver<()>,
    pending: Cell<usize>,
    answers: Arc<Mutex<TcpStream>>,
    game: Arc<(Mutex<Option<Content>>, Condvar)>,
}

impl SocketProbe {
    pub fn connect(driver: SocketAddr) -> SocketProbe {
        let stream = TcpStream::connect(driver)
            .unwrap_or_else(|error| panic!("the probe's driver at {driver}: {error}"));
        let questions = stream
            .try_clone()
            .expect("a second handle on the probe socket");
        let answers = Arc::new(Mutex::new(stream));
        let game: Arc<(Mutex<Option<Content>>, Condvar)> = Arc::default();
        let (ask, asked) = channel();
        let (thread_answers, thread_game) = (answers.clone(), game.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(questions).lines().map_while(Result::ok) {
                if probe::asks_for_snapshot(&line) {
                    if ask.send(()).is_err() {
                        return;
                    }
                    continue;
                }
                let content = loaded(&thread_game);
                let question = probe::read_question(&content, &line)
                    .unwrap_or_else(|error| panic!("the probe's driver asked {line}: {error}"));
                let answer = probe::answer(&content, &question).expect("a question about the game");
                let mut answers = thread_answers.lock().expect("the probe's answers");
                if writeln!(answers, "{answer}").is_err() {
                    return;
                }
            }
        });
        SocketProbe {
            asked,
            pending: Cell::new(0),
            answers,
            game,
        }
    }
}

fn loaded(game: &(Mutex<Option<Content>>, Condvar)) -> Content {
    let (content, known) = game;
    let content = known
        .wait_while(content.lock().expect("the probe's game"), |content| {
            content.is_none()
        })
        .expect("the probe's game");
    content.clone().expect("a loaded game")
}

impl ProbeChannel for SocketProbe {
    fn awaited(&self) -> bool {
        self.pending
            .set(self.pending.get() + self.asked.try_iter().count());
        self.pending.get() > 0
    }

    // A driver that hung up wants no more answers.
    fn deliver(&self, snapshot: &str) {
        let mut answers = self.answers.lock().expect("the probe's answers");
        for _ in 0..self.pending.replace(0) {
            let _ = writeln!(answers, "{snapshot}");
        }
    }

    fn know(&self, content: &Content) {
        let (game, known) = &*self.game;
        *game.lock().expect("the probe's game") = Some(content.clone());
        known.notify_all();
    }
}

fn refused(response: &mut ureq::http::Response<ureq::Body>) -> Result<(), FetchFailure> {
    if response.status().is_success() {
        return Ok(());
    }
    Err(FetchFailure::Status {
        status: response.status().as_u16(),
        body: response.body_mut().read_to_string().unwrap_or_default(),
    })
}

struct ThreadSocket {
    incoming: Mutex<Receiver<Vec<u8>>>,
    outgoing: Sender<Vec<u8>>,
    state: Arc<Mutex<SocketState>>,
}

impl ThreadSocket {
    fn open(url: &str) -> ThreadSocket {
        let (incoming_tx, incoming) = channel();
        let (outgoing, outgoing_rx) = channel();
        let state = Arc::new(Mutex::new(SocketState::Connecting));
        std::thread::spawn({
            let state = state.clone();
            let url = url.to_owned();
            move || drive(&url, &state, &incoming_tx, &outgoing_rx)
        });
        ThreadSocket {
            incoming: Mutex::new(incoming),
            outgoing,
            state,
        }
    }
}

impl ServerSocket for ThreadSocket {
    fn recv(&self) -> Option<Vec<u8>> {
        self.incoming.lock().expect("socket lock").try_recv().ok()
    }

    fn send(&self, packet: &[u8]) {
        if self.state() == SocketState::Open {
            let _ = self.outgoing.send(packet.to_vec());
        }
    }

    fn state(&self) -> SocketState {
        *self.state.lock().expect("socket state lock")
    }
}

fn drive(
    url: &str,
    state: &Mutex<SocketState>,
    incoming: &Sender<Vec<u8>>,
    outgoing: &Receiver<Vec<u8>>,
) {
    let set = |to: SocketState| *state.lock().expect("socket state lock") = to;
    let Ok((mut ws, _)) = tungstenite::connect(url).map_err(|error| eprintln!("{url}: {error}"))
    else {
        return set(SocketState::Closed);
    };
    let polling = match ws.get_ref() {
        MaybeTlsStream::Plain(tcp) => tcp.set_read_timeout(Some(READ_POLL)),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(Some(READ_POLL)),
        _ => Ok(()),
    };
    if polling.is_err() {
        return set(SocketState::Closed);
    }
    set(SocketState::Open);
    let closed = || set(SocketState::Closed);
    loop {
        loop {
            match outgoing.try_recv() {
                Ok(packet) => match ws.send(Message::Binary(packet.into())) {
                    Ok(()) => {}
                    Err(tungstenite::Error::Io(error)) if retryable(&error) => {}
                    Err(_) => return closed(),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return closed(),
            }
        }
        let _ = ws.flush();
        match ws.read() {
            Ok(Message::Binary(data)) => {
                if incoming.send(data.to_vec()).is_err() {
                    return closed();
                }
            }
            Ok(Message::Close(_)) => return closed(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if retryable(&error) => {}
            Err(_) => return closed(),
        }
    }
}

// A socket call with a timeout fails as interrupted when the process is stopped and resumed (a
// debugger, a suspended terminal job), so that is no more the connection's end than a poll that
// found nothing.
fn retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

const READ_POLL: Duration = Duration::from_millis(1);
