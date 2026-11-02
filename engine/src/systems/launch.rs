use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::content::version;
use crate::core::net::auth::TokenClaims;
use crate::core::time::UtcHour;
use crate::systems::player::SpawnPolicy;
use crate::systems::probe::ProbeFrameQuestion;
use crate::systems::script::Place;
use crate::systems::staging::Staging;

pub const BUILD_INFO_FLAG: &str = "--build-info";
pub const SERVER_ENV_PREFIX: &str = "RIFT_GAME_SERVER_";
pub const CLIENT_ENV_PREFIX: &str = "RIFT_CLIENT_";
pub const LAUNCH_FILE: &str = "rift.env";
pub const TEST_CHARACTER_ROUTE: &str = "/test/character";
/// The histogram of how long the server takes to step every world one tick.
pub const TICK_METRIC: &str = "rift_tick_duration_seconds";

/// What a server on fake auth takes as an account's sign-in.
pub fn fake_token(account: &str) -> String {
    TokenClaims {
        sub: account.to_owned(),
        preferred_username: Some(account.to_owned()),
        name: None,
    }
    .unsigned()
}

/// The JSON Schema of what a game server speaks to whoever runs it: the line it prints once
/// ready, and what its test routes take.
pub fn server_schema() -> String {
    let schema = schemars::schema_for!(ServerProtocol);
    serde_json::to_string_pretty(&schema).expect("a serializable schema")
}

/// Everything a game server speaks to whoever runs it, for its schema.
#[derive(JsonSchema)]
pub struct ServerProtocol {
    pub ready: ReadyLine,
    pub test_character: TestCharacterRequest,
    pub stage: Staging,
}

/// What a binary starts with: its environment, over the [`LAUNCH_FILE`] beside it.
pub struct LaunchSettings {
    folder: PathBuf,
    vars: BTreeMap<String, String>,
}

impl LaunchSettings {
    pub fn of_this_binary() -> LaunchSettings {
        let binary = std::env::current_exe().expect("the binary's own path");
        let folder = binary.parent().expect("the binary's folder");
        LaunchSettings::read(folder, std::env::vars())
    }

    pub fn read(
        folder: &Path,
        environment: impl IntoIterator<Item = (String, String)>,
    ) -> LaunchSettings {
        let file = folder.join(LAUNCH_FILE);
        let mut vars = BTreeMap::new();
        match dotenvy::from_path_iter(&file) {
            Ok(lines) => {
                for line in lines {
                    let (key, value) =
                        line.unwrap_or_else(|error| panic!("{}: {error}", file.display()));
                    vars.insert(key, value);
                }
            }
            Err(error) if error.not_found() => {}
            Err(error) => panic!("{}: {error}", file.display()),
        }
        vars.extend(environment);
        LaunchSettings {
            folder: folder.to_path_buf(),
            vars,
        }
    }

    pub fn get<T: DeserializeOwned>(&self, prefix: &str) -> T {
        envy::prefixed(prefix)
            .from_iter(self.vars.clone())
            .unwrap_or_else(|error| {
                panic!(
                    "{prefix}* settings, from the environment or {}: {error}",
                    self.folder.join(LAUNCH_FILE).display()
                )
            })
    }

    /// Relative to the binary's folder, so a binary finds its files however it's started.
    pub fn path(&self, path: &Path) -> PathBuf {
        self.folder.join(path)
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildInfo {
    pub build: String,
    pub content_version: String,
}

impl BuildInfo {
    pub fn current() -> BuildInfo {
        BuildInfo {
            build: version::build().to_owned(),
            content_version: super::registry().version().0.clone(),
        }
    }
}

/// What a started server prints on stdout, once players can connect.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ReadyLine {
    pub ready: ServerReady,
    pub build: BuildInfo,
}

/// The ports a started server listens on, announced once players can connect.
#[derive(Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct ServerReady {
    pub http: u16,
    pub udp: u16,
}

/// What a server that hands out test characters takes at [`TEST_CHARACTER_ROUTE`], from the
/// account whose next sign-in plays the character.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TestCharacterRequest {
    pub test_character: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Place>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ServerConfig {
    pub bind: IpAddr,
    pub port: u16,
    /// The first of the ports players' clients connect to play at, one for every 1024 of
    /// `max_clients`, each the port after the last; 0 takes any free ones.
    pub udp_port: u16,
    /// The address players reach this machine at, which the connect tokens it hands out name, so
    /// a client that came by another address can't connect.
    pub public_ip: IpAddr,
    pub max_clients: usize,
    pub content_dir: PathBuf,
    pub player_spawn_policy: SpawnPolicy,
    pub player_immortal: bool,
    pub daylight: ServerDaylight,
    pub rng_seed: Option<u64>,
    pub daily_reset_utc_hour: UtcHour,
    pub auth: AuthMode,
    pub test_characters: bool,
    pub exit_on_stdin_eof: bool,
}

impl ServerConfig {
    /// A server for a tool on this machine to play `content_dir` on alone: on any free ports of
    /// the loopback, signing players in without asking, handing out test characters and stopping
    /// with the tool that started it.
    pub fn local(content_dir: PathBuf) -> ServerConfig {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        ServerConfig {
            bind: loopback,
            port: 0,
            udp_port: 0,
            public_ip: loopback,
            max_clients: 8,
            content_dir,
            player_spawn_policy: SpawnPolicy::Map,
            player_immortal: false,
            daylight: ServerDaylight::Turning,
            rng_seed: None,
            daily_reset_utc_hour: UtcHour::MIDNIGHT,
            auth: AuthMode::Fake,
            test_characters: true,
            exit_on_stdin_eof: true,
        }
    }
}

/// Whether a game's day turns on the server, or holds at its start hour, as tests that compare how
/// the world looks need it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerDaylight {
    Turning,
    Still,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Oidc,
    Fake,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ClientConfig {
    pub game_server_url: String,
    pub auth: AuthMode,
    /// Signs in as this token, without asking the player.
    pub access_token: Option<String>,
    pub profile_dir: PathBuf,
    pub exit_on_stdin_eof: bool,
    pub probe: Option<SocketAddr>,
}

pub fn env_of<T: Serialize>(prefix: &str, config: &T) -> Vec<(String, String)> {
    let value = serde_json::to_value(config).expect("a flat config");
    let fields = value.as_object().expect("a flat config");
    fields
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(field, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (format!("{prefix}{}", field.to_ascii_uppercase()), text)
        })
        .collect()
}

/// Starts the game server `binary` with `envs` and waits until it prints its ready line, echoing
/// whatever else it prints. The server is killed when it doesn't get ready within `ready_within`.
pub fn start_game_server(
    binary: &Path,
    envs: impl IntoIterator<Item = (String, String)>,
    ready_within: Duration,
) -> Result<(Child, ServerReady), String> {
    let mut process = Command::new(binary)
        .envs(envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{} doesn't run: {error}", binary.display()))?;
    let stdout = process.stdout.take().expect("a piped stdout");
    let (ready, readied) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match serde_json::from_str::<ReadyLine>(&line) {
                Ok(line) => {
                    let _ = ready.send(line.ready);
                }
                Err(_) => println!("[game-server] {line}"),
            }
        }
    });
    match readied.recv_timeout(ready_within) {
        Ok(ready) => Ok((process, ready)),
        Err(_) => {
            let _ = process.kill();
            let _ = process.wait();
            Err(format!("{} never got ready", binary.display()))
        }
    }
}

/// The driver of a desktop client's probe: asks it its frame questions and reads its answers.
pub struct ProbeDriver {
    questions: TcpStream,
    answers: BufReader<TcpStream>,
}

impl ProbeDriver {
    /// Waits for the client's probe to connect to `listener`.
    pub fn accept(listener: &TcpListener) -> std::io::Result<ProbeDriver> {
        let (stream, _) = listener.accept()?;
        let answers = BufReader::new(stream.try_clone()?);
        Ok(ProbeDriver {
            questions: stream,
            answers,
        })
    }

    /// The probe's answer, one line of JSON, to `question`.
    pub fn ask(&mut self, question: &ProbeFrameQuestion) -> Result<String, String> {
        let question = serde_json::to_string(question).expect("a question serializes");
        writeln!(self.questions, "{question}")
            .map_err(|error| format!("the client takes no question: {error}"))?;
        let mut answer = String::new();
        match self.answers.read_line(&mut answer) {
            Ok(0) => Err("the client hung up".to_owned()),
            Ok(_) => Ok(answer),
            Err(error) => Err(format!("the client stopped answering: {error}")),
        }
    }
}

pub fn build_info_line() -> String {
    serde_json::to_string(&BuildInfo::current()).expect("a build info line")
}
