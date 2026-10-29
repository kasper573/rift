use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::content::version;
use crate::core::net::auth::TokenClaims;
use crate::core::time::UtcHour;
use crate::systems::player::SpawnPolicy;
use crate::systems::script::Place;
use crate::systems::staging::Staging;

pub const BUILD_INFO_FLAG: &str = "--build-info";
pub const SERVER_ENV_PREFIX: &str = "RIFT_GAME_SERVER_";
pub const CLIENT_ENV_PREFIX: &str = "RIFT_CLIENT_";
pub const SETTINGS_FILE: &str = "rift.env";
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

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ServerProtocol {
    ready: ReadyLine,
    test_character: TestCharacterRequest,
    stage: Staging,
}

/// What a binary starts with: its environment, over the [`SETTINGS_FILE`] beside it.
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
        let file = folder.join(SETTINGS_FILE);
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
                    self.folder.join(SETTINGS_FILE).display()
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
    pub ws: u16,
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
    pub ws_port: u16,
    pub max_clients: usize,
    pub content_dir: PathBuf,
    pub player_spawn_policy: SpawnPolicy,
    pub player_immortal: bool,
    pub rng_seed: Option<u64>,
    pub daily_reset_utc_hour: UtcHour,
    pub auth: AuthMode,
    pub test_characters: bool,
    pub exit_on_stdin_eof: bool,
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
    pub game_server_ws_url: String,
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

pub fn build_info_line() -> String {
    serde_json::to_string(&BuildInfo::current()).expect("a build info line")
}
