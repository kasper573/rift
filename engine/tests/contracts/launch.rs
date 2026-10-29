use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use engine::core::time::UtcHour;
use engine::systems::launch::{
    AuthMode, CLIENT_ENV_PREFIX, ClientConfig, LaunchSettings, SERVER_ENV_PREFIX, SETTINGS_FILE,
    ServerConfig, env_of,
};
use engine::systems::player::SpawnPolicy;

fn server_config(rng_seed: Option<u64>) -> ServerConfig {
    ServerConfig {
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 4000,
        ws_port: 4001,
        max_clients: 8,
        content_dir: PathBuf::from("/games/example"),
        player_spawn_policy: SpawnPolicy::Dist,
        player_immortal: true,
        rng_seed,
        daily_reset_utc_hour: UtcHour::try_from(5).expect("an hour of the day"),
        auth: AuthMode::Fake,
        test_characters: true,
        exit_on_stdin_eof: true,
    }
}

fn read_back<T: serde::de::DeserializeOwned>(prefix: &str, config: &impl serde::Serialize) -> T {
    envy::prefixed(prefix)
        .from_iter(env_of(prefix, config))
        .expect("the env written for a config reads back")
}

#[test]
fn a_binarys_config_reads_back_from_the_env_written_for_it() {
    for config in [server_config(Some(7)), server_config(None)] {
        assert_eq!(
            read_back::<ServerConfig>(SERVER_ENV_PREFIX, &config),
            config
        );
    }
    let config = ClientConfig {
        game_server_url: "http://127.0.0.1:4000".to_owned(),
        game_server_ws_url: "ws://127.0.0.1:4001".to_owned(),
        auth: AuthMode::Oidc,
        access_token: Some("a.b.c".to_owned()),
        profile_dir: PathBuf::from("/profiles/tester"),
        exit_on_stdin_eof: false,
        probe: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000)),
    };
    assert_eq!(
        read_back::<ClientConfig>(CLIENT_ENV_PREFIX, &config),
        config
    );
}

fn client_config() -> ClientConfig {
    ClientConfig {
        game_server_url: "https://game-server.example".to_owned(),
        game_server_ws_url: "wss://game-server.example".to_owned(),
        auth: AuthMode::Oidc,
        access_token: None,
        profile_dir: PathBuf::from("profile"),
        exit_on_stdin_eof: false,
        probe: None,
    }
}

fn settings_file(folder: &Path, config: &ClientConfig) {
    let lines: Vec<String> = env_of(CLIENT_ENV_PREFIX, config)
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    std::fs::write(folder.join(SETTINGS_FILE), lines.join("\n")).expect("a settings file");
}

#[test]
fn a_binary_takes_its_settings_from_the_file_beside_it_or_its_environment_alone() {
    let folder = tempfile::tempdir().expect("a folder");
    settings_file(folder.path(), &client_config());
    let settings = LaunchSettings::read(folder.path(), []);
    assert_eq!(
        settings.get::<ClientConfig>(CLIENT_ENV_PREFIX),
        client_config()
    );

    let without_a_file = tempfile::tempdir().expect("a folder");
    let settings = LaunchSettings::read(
        without_a_file.path(),
        env_of(CLIENT_ENV_PREFIX, &client_config()),
    );
    assert_eq!(
        settings.get::<ClientConfig>(CLIENT_ENV_PREFIX),
        client_config()
    );
}

#[test]
fn the_environment_overrides_the_settings_file() {
    let folder = tempfile::tempdir().expect("a folder");
    settings_file(folder.path(), &client_config());
    let settings = LaunchSettings::read(
        folder.path(),
        [(format!("{CLIENT_ENV_PREFIX}AUTH"), "fake".to_owned())],
    );
    assert_eq!(
        settings.get::<ClientConfig>(CLIENT_ENV_PREFIX),
        ClientConfig {
            auth: AuthMode::Fake,
            ..client_config()
        }
    );
}

#[test]
fn a_relative_path_is_relative_to_the_binarys_folder() {
    let folder = tempfile::tempdir().expect("a folder");
    let settings = LaunchSettings::read(folder.path(), []);
    assert_eq!(
        settings.path(Path::new("content")),
        folder.path().join("content")
    );
    let elsewhere = tempfile::tempdir().expect("another folder");
    assert_eq!(settings.path(elsewhere.path()), elsewhere.path());
}
