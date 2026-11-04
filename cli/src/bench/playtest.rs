use std::fs::File;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use engine::core::content::{Content, ContentScope, with_content};
use engine::core::os::Profile;
use engine::core::render::video::VideoPreferences;
use engine::core::units::Bytes;
use engine::systems::launch::{
    self, AuthMode, CLIENT_ENV_PREFIX, ClientConfig, ProbeDriver, TEST_CHARACTER_ROUTE,
    TestCharacterRequest,
};
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::probe::{ProbeFrameQuestion, ProbeSnapshot};
use engine::systems::staging::{STAGING_ROUTE, Staging};
use engine::systems::{GameSettings, settings};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tempfile::TempDir;

use super::desktop::BenchDesktop;
use crate::binaries::{self, DESKTOP_CLIENT, GAME_SERVER};
use crate::server::{GameServer, LOCALHOST, bench_config, resident};

const ACCOUNT: &str = "bench";
const CLIENT_LOG: &str = "client.log";
const LOG_TAIL: usize = 40;

pub struct Playtest {
    content: Content,
    server: GameServer,
    client: Child,
    probe: ProbeDriver,
    desktop: BenchDesktop,
    folder: TempDir,
}

impl Playtest {
    /// The game's first test character at the game's initial spawn, or a fresh character where the
    /// game has none, in a client on a desktop of the bench's own, shown as `video` asks. Refused
    /// when the server or the client doesn't come up.
    pub fn start(
        game: &Path,
        content: &Content,
        video: VideoPreferences,
    ) -> Result<Playtest, String> {
        let folder = tempfile::tempdir().expect("a folder for the client's profile and log");
        settings::save_video(&Profile(folder.path().join("profile")), video);
        let server = GameServer::start(&binaries::beside(GAME_SERVER), &bench_config(game));
        if let Some(character) = content.ids::<CharacterSnapshot>().next() {
            let request = TestCharacterRequest {
                test_character: character.path(content).to_owned(),
                start: Some(content.built::<GameSettings>().initial_spawn),
            };
            post(&server, content, TEST_CHARACTER_ROUTE, &request)
                .map_err(|problem| format!("the server refused the test character: {problem}"))?;
        }

        let listener = TcpListener::bind((LOCALHOST, 0)).expect("a probe port");
        let config = ClientConfig {
            game_server_url: server.http_url(),
            auth: AuthMode::Fake,
            access_token: Some(launch::fake_token(ACCOUNT)),
            profile_dir: folder.path().join("profile"),
            exit_on_stdin_eof: true,
            probe: Some(listener.local_addr().expect("a bound probe port")),
        };
        let desktop = BenchDesktop::open(folder.path());
        let log = File::create(folder.path().join(CLIENT_LOG)).expect("a client log");
        let mut client = Command::new(binaries::beside(DESKTOP_CLIENT))
            .envs(launch::env_of(CLIENT_ENV_PREFIX, &config))
            .envs(desktop.env())
            .stdin(Stdio::piped())
            .stdout(log.try_clone().expect("a second handle on the client log"))
            .stderr(log)
            .spawn()
            .map_err(|error| format!("the desktop client doesn't run: {error}"))?;
        let probe = match ProbeDriver::accept(&listener, &mut client, WAIT) {
            Ok(probe) => probe,
            Err(problem) => {
                let _ = client.kill();
                let _ = client.wait();
                return Err(format!(
                    "{problem}; its log is {}",
                    folder.path().join(CLIENT_LOG).display()
                ));
            }
        };
        desktop.seat(client.id());
        Ok(Playtest {
            content: content.clone(),
            server,
            client,
            probe,
            desktop,
            folder,
        })
    }

    /// What the client shows in a frame after asking; a client that doesn't answer ends the bench.
    pub fn snapshot(&mut self) -> ProbeSnapshot {
        self.ask(&ProbeFrameQuestion::Snapshot)
            .unwrap_or_else(|problem| panic!("{problem}; its log ends:\n{}", self.log_tail()))
    }

    /// The client's answer to `question`, or why it gave none.
    pub fn ask<Answer: DeserializeOwned>(
        &mut self,
        question: &ProbeFrameQuestion,
    ) -> Result<Answer, String> {
        let answer = self.probe.ask(question)?;
        in_scope(&self.content, || serde_json::from_str(&answer))
            .map_err(|error| format!("an unreadable answer to {question:?}: {error}"))
    }

    /// How the client exited, once it has.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.client.try_wait().ok().flatten()
    }

    pub fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.folder.path().join(CLIENT_LOG)).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(LOG_TAIL)..].join("\n")
    }

    /// Asks the server to stage `staging` for the character, and tells why it couldn't.
    pub fn stage(&self, staging: &Staging) -> Result<(), String> {
        post(&self.server, &self.content, STAGING_ROUTE, staging)
    }

    pub fn wait_until(
        &mut self,
        what: &str,
        mut done: impl FnMut(&ProbeSnapshot) -> bool,
    ) -> ProbeSnapshot {
        let deadline = Instant::now() + WAIT;
        loop {
            let snapshot = self.snapshot();
            if done(&snapshot) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "gave up waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Lets the client play on for `lasting` without asking it anything, so what it does meanwhile
    /// is drawing, not answering.
    pub fn linger(&mut self, lasting: Duration) {
        std::thread::sleep(lasting);
    }

    /// The X display the client's window is on.
    pub fn display(&self) -> &str {
        self.desktop.display()
    }

    pub fn client_memory(&self) -> Bytes {
        resident(self.client.id())
    }

    pub fn server_memory(&self) -> Bytes {
        self.server.memory()
    }

    pub fn content(&self) -> &Content {
        &self.content
    }
}

impl Drop for Playtest {
    fn drop(&mut self) {
        let _ = self.client.kill();
        let _ = self.client.wait();
    }
}

const WAIT: Duration = Duration::from_secs(120);

fn post(
    server: &GameServer,
    content: &Content,
    route: &str,
    body: &impl Serialize,
) -> Result<(), String> {
    let body = in_scope(content, || serde_json::to_string(body)).expect("a serializable request");
    let mut response = ureq::post(format!("{}{route}", server.http_url()))
        .header("Authorization", launch::fake_token(ACCOUNT))
        .config()
        .http_status_as_error(false)
        .build()
        .send(body)
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        return Ok(());
    }
    Err(response.body_mut().read_to_string().unwrap_or_default())
}

/// Content ids travel by path, so JSON is read and written within the content's scope.
fn in_scope<T>(content: &Content, f: impl FnOnce() -> T) -> T {
    with_content(ContentScope::of(content), f).0
}
