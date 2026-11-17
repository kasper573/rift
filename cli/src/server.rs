use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use engine::core::units::{Bytes, HourOfDay};
use engine::systems::launch::{
    self, SERVER_ENV_PREFIX, ServerConfig, ServerDaylight, ServerReady, ServerWeather,
};
use tempfile::TempDir;

pub const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

// Runs play out alike, so they compare.
pub const SEED: u64 = 1931;
/// Room on a bench's server for its own client and the other players a scenario brings in.
pub const BENCH_CLIENTS: usize = 64;

/// A server playing `game` on any free ports, as the benches run it: seeded, its players
/// immortal and signed in without asking, its day held at noon and its weather at each area's
/// first spell, so runs compare, saving into `run`. A bench says what else it needs.
pub fn bench_config(game: &Path, run: &Path) -> ServerConfig {
    ServerConfig {
        max_clients: BENCH_CLIENTS,
        characters_immortal: true,
        daylight: ServerDaylight::Fixed,
        hour: Some(HourOfDay::NOON),
        weather: ServerWeather::Still,
        rng_seed: Some(SEED),
        ..ServerConfig::local(absolute(game), run)
    }
}

/// A game server process, stopped when dropped.
pub struct GameServer {
    process: Child,
    ready: ServerReady,
    /// The folder it saves into, gone with it.
    _run: TempDir,
}

impl GameServer {
    /// The server `config` describes, given the folder that goes with its run.
    pub fn start(binary: &Path, config: impl FnOnce(&Path) -> ServerConfig) -> GameServer {
        let run = tempfile::tempdir().expect("a folder for what the server saves");
        let (process, ready) = launch::start_game_server(
            binary,
            launch::env_of(SERVER_ENV_PREFIX, &config(run.path())),
            READY_WITHIN,
        )
        .unwrap_or_else(|problem| panic!("{problem}"));
        GameServer {
            process,
            ready,
            _run: run,
        }
    }

    pub fn id(&self) -> u32 {
        self.process.id()
    }

    pub fn http_url(&self) -> String {
        format!("http://{LOCALHOST}:{}", self.ready.http)
    }

    pub fn memory(&self) -> Bytes {
        resident(self.process.id())
    }
}

impl Drop for GameServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// The server reads a relative path against its own binary's folder, not rift's.
pub fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).expect("a path the server can find")
}

/// The memory a running process holds resident, as the system sees it.
pub fn resident(process: u32) -> Bytes {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let pid = Pid::from_u32(process);
    let mut system = System::new().expect("the system's processes to read");
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );
    Bytes(
        system
            .process(pid)
            .expect("the process is running")
            .memory(),
    )
}

const READY_WITHIN: Duration = Duration::from_secs(180);
