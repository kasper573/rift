use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use engine::core::content::{Content, ContentScope, with_content};
use engine::core::time::UtcHour;
use engine::core::units::Bytes;
use engine::systems::GameSettings;
use engine::systems::launch::{
    self, AuthMode, CLIENT_ENV_PREFIX, ClientConfig, ServerConfig, TEST_CHARACTER_ROUTE,
    TestCharacterRequest,
};
use engine::systems::player::SpawnPolicy;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::probe::{ProbeQuestion, ProbeSnapshot};
use engine::systems::staging::{STAGING_ROUTE, Staging};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tempfile::TempDir;

use super::desktop::BenchDesktop;
use super::sim::SEED;
use crate::binaries::{self, DESKTOP_CLIENT, GAME_SERVER};
use crate::server::{GameServer, LOCALHOST, absolute, resident};

const ACCOUNT: &str = "bench";

pub struct Playtest {
    content: Content,
    server: GameServer,
    client: Child,
    questions: TcpStream,
    answers: BufReader<TcpStream>,
    _desktop: BenchDesktop,
    _folder: TempDir,
}

impl Playtest {
    /// The game's first test character, at the game's initial spawn, in a client on a desktop of the
    /// bench's own.
    pub fn start(game: &Path, content: &Content) -> Playtest {
        let folder = tempfile::tempdir().expect("a folder for the client's profile and log");
        let server = GameServer::start(
            &binaries::beside(GAME_SERVER),
            &ServerConfig {
                bind: LOCALHOST,
                port: 0,
                ws_port: 0,
                max_clients: 8,
                content_dir: absolute(game),
                player_spawn_policy: SpawnPolicy::Map,
                player_immortal: true,
                rng_seed: Some(SEED),
                daily_reset_utc_hour: UtcHour::MIDNIGHT,
                auth: AuthMode::Fake,
                test_characters: true,
                exit_on_stdin_eof: true,
            },
        );
        let character = content
            .ids::<CharacterSnapshot>()
            .next()
            .expect("the game has a test character to play the bench as");
        let request = TestCharacterRequest {
            test_character: character.path(content).to_owned(),
            start: Some(content.built::<GameSettings>().initial_spawn),
        };
        post(&server, content, TEST_CHARACTER_ROUTE, &request)
            .unwrap_or_else(|problem| panic!("the server refused the test character: {problem}"));

        let listener = TcpListener::bind((LOCALHOST, 0)).expect("a probe port");
        let config = ClientConfig {
            game_server_url: server.http_url(),
            game_server_ws_url: server.ws_url(),
            auth: AuthMode::Fake,
            access_token: Some(launch::fake_token(ACCOUNT)),
            profile_dir: folder.path().join("profile"),
            exit_on_stdin_eof: true,
            probe: Some(listener.local_addr().expect("a bound probe port")),
        };
        let desktop = BenchDesktop::open(folder.path());
        let log = File::create(folder.path().join("client.log")).expect("a client log");
        let client = Command::new(binaries::beside(DESKTOP_CLIENT))
            .envs(launch::env_of(CLIENT_ENV_PREFIX, &config))
            .envs(desktop.env())
            .stdin(Stdio::piped())
            .stdout(log.try_clone().expect("a second handle on the client log"))
            .stderr(log)
            .spawn()
            .expect("the desktop client runs");
        let (questions, _) = listener.accept().expect("the client's probe connects");
        desktop.seat(client.id());
        let answers = BufReader::new(questions.try_clone().expect("a second handle on the probe"));
        Playtest {
            content: content.clone(),
            server,
            client,
            questions,
            answers,
            _desktop: desktop,
            _folder: folder,
        }
    }

    pub fn snapshot(&mut self) -> ProbeSnapshot {
        self.ask(&ProbeQuestion::Snapshot)
    }

    pub fn ask<Answer: DeserializeOwned>(&mut self, question: &ProbeQuestion) -> Answer {
        let line = in_scope(&self.content, || serde_json::to_string(question))
            .expect("a serializable question");
        writeln!(self.questions, "{line}").expect("the client takes questions");
        let mut answer = String::new();
        match self.answers.read_line(&mut answer) {
            Ok(0) => panic!("the client hung up"),
            Ok(_) => {}
            Err(error) => panic!("the client stopped answering: {error}"),
        }
        in_scope(&self.content, || serde_json::from_str(&answer))
            .unwrap_or_else(|error| panic!("an unreadable answer to {line}: {error}"))
    }

    /// Asks the server to stage `staging` for the character, and tells why it couldn't.
    pub fn stage(&self, staging: Staging) -> Result<(), String> {
        post(&self.server, &self.content, STAGING_ROUTE, &staging)
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
