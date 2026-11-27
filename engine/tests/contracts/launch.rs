use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use engine::core::store::RecordStoreAddress;
use engine::core::time::UtcHour;
use engine::core::units::Seconds;
use engine::systems::launch::{
    AuthMode, CLIENT_ENV_PREFIX, ClientConfig, LAUNCH_FILE, LaunchSettings, SERVER_ENV_PREFIX,
    ServerConfig, ServerDaylight, ServerWeather, env_of,
};
use engine::systems::player::SpawnPolicy;

fn server_config(rng_seed: Option<u64>) -> ServerConfig {
    ServerConfig {
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 4000,
        udp_port: 4001,
        public_ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
        max_clients: 8,
        max_instances: 4,
        content_dir: PathBuf::from("/games/example"),
        character_spawn_policy: SpawnPolicy::Dist,
        characters_immortal: true,
        daylight: ServerDaylight::Still,
        hour: None,
        weather: ServerWeather::Still,
        rng_seed,
        daily_reset_utc_hour: UtcHour::try_from(5).expect("an hour of the day"),
        auth: AuthMode::Fake,
        test_characters: true,
        exit_on_stdin_eof: true,
        store: RecordStoreAddress::new("postgres://game:secret@database/records"),
        checkpoint_every: Seconds(30.0),
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
        auth: AuthMode::Oidc,
        access_token: Some("a.b.c".to_owned()),
        profile_dir: PathBuf::from("/profiles/tester"),
        exit_on_stdin_eof: false,
        probe: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000)),
        releases_url: Some("https://downloads.example/releases".to_owned()),
        updated_from: Some("2026.11.14-9c2e1a07".to_owned()),
    };
    assert_eq!(
        read_back::<ClientConfig>(CLIENT_ENV_PREFIX, &config),
        config
    );
}

fn client_config() -> ClientConfig {
    ClientConfig {
        game_server_url: "https://game-server.example".to_owned(),
        auth: AuthMode::Oidc,
        access_token: None,
        profile_dir: PathBuf::from("profile"),
        exit_on_stdin_eof: false,
        probe: None,
        releases_url: None,
        updated_from: None,
    }
}

fn settings_file(folder: &Path, config: &ClientConfig) {
    let lines: Vec<String> = env_of(CLIENT_ENV_PREFIX, config)
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    std::fs::write(folder.join(LAUNCH_FILE), lines.join("\n")).expect("a settings file");
}

#[test]
fn a_binary_takes_its_settings_from_the_file_beside_it_its_environment_over_it_and_paths_from_its_folder()
 {
    let folder = tempfile::tempdir().expect("a folder");
    settings_file(folder.path(), &client_config());
    let settings = LaunchSettings::read(folder.path(), []);
    assert_eq!(
        settings.get::<ClientConfig>(CLIENT_ENV_PREFIX),
        client_config()
    );
    assert_eq!(
        settings.path(Path::new("content")),
        folder.path().join("content"),
        "a relative path is relative to the binary's folder"
    );
    let elsewhere = tempfile::tempdir().expect("another folder");
    assert_eq!(settings.path(elsewhere.path()), elsewhere.path());

    let overridden = LaunchSettings::read(
        folder.path(),
        [(format!("{CLIENT_ENV_PREFIX}AUTH"), "fake".to_owned())],
    );
    assert_eq!(
        overridden.get::<ClientConfig>(CLIENT_ENV_PREFIX),
        ClientConfig {
            auth: AuthMode::Fake,
            ..client_config()
        },
        "the environment overrides the file"
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
