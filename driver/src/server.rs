use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use engine::core::time::UtcHour;
use engine::systems::launch::{
    self, AuthMode, ReadyLine, SERVER_ENV_PREFIX, ServerConfig, ServerReady, Switch,
    TEST_CHARACTER_ROUTE, TestCharacterRequest,
};
use engine::systems::player::SpawnPolicy;
use engine::systems::script::Place;

use crate::content::to_json;

// The game server binary, run natively on fake auth so any account signs in with roles of its choice.
pub struct Server {
    process: Child,
    binary: PathBuf,
    config: ServerConfig,
    ready: ServerReady,
    log: PathBuf,
}

impl Server {
    pub fn start(binary: &Path, content: &Path, seed: Option<u64>, log: &Path) -> Server {
        let config = ServerConfig {
            bind: LOCALHOST,
            port: 0,
            ws_port: 0,
            max_clients: 8,
            content_dir: content.to_path_buf(),
            player_spawn_type: SpawnPolicy::Map,
            // A playtest plays its story through: monsters met on the way wear players down, never
            // out.
            player_immortal: true,
            rng_seed: seed,
            daily_reset_utc_hour: UtcHour::MIDNIGHT,
            auth: AuthMode::Fake,
            test_characters: Switch::On,
            exit_on_stdin_eof: true,
            pyroscope_enabled: false,
            pyroscope_sample_hz: 100,
        };
        let (process, ready) = run(binary, &config, log);
        Server {
            process,
            binary: binary.to_path_buf(),
            config,
            ready,
            log: log.to_path_buf(),
        }
    }

    pub fn http_url(&self) -> String {
        format!("http://{LOCALHOST}:{}", self.ready.http)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{LOCALHOST}:{}", self.ready.ws)
    }

    // The account's next sign-in plays the test character, starting where it says or where it would.
    pub fn hand_over(&self, account: &str, character: &str, start: Option<Place>) {
        let request = TestCharacterRequest {
            test_character: character.to_owned(),
            start,
        };
        ureq::post(format!("{}{TEST_CHARACTER_ROUTE}", self.http_url()))
            .header("Authorization", launch::fake_token(account))
            .send(to_json(&request))
            .unwrap_or_else(|error| panic!("the game server refused {character}: {error}"));
    }

    // A deploy: the same ports, so connected clients find the server again, on the content in `folder`.
    // `while_down` runs while there is no server to find.
    pub fn restart_on(&mut self, folder: &Path, while_down: impl FnOnce()) {
        self.stop();
        while_down();
        self.config.content_dir = folder.to_path_buf();
        self.config.port = self.ready.http;
        self.config.ws_port = self.ready.ws;
        (self.process, self.ready) = run(&self.binary, &self.config, &self.log);
    }

    pub fn stop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
// Generous: a playtest shares its machine with the other playtests' clients and recorders, and a
// starved server loads its content slowly.
const READY_WITHIN: Duration = Duration::from_secs(180);

fn run(binary: &Path, config: &ServerConfig, log: &Path) -> (Child, ServerReady) {
    let mut log: File = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("a server log");
    let mut process = Command::new(binary)
        .envs(launch::env_of(SERVER_ENV_PREFIX, config))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log.try_clone().expect("a second handle on the server log"))
        .spawn()
        .expect("the game server binary runs");
    let stdout = process.stdout.take().expect("a piped stdout");
    let (ready, readied) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(line) = serde_json::from_str::<ReadyLine>(&line) {
                let _ = ready.send(line.ready);
            }
            let _ = writeln!(log, "{line}");
        }
    });
    let ready = readied
        .recv_timeout(READY_WITHIN)
        .expect("the game server gets ready");
    (process, ready)
}
