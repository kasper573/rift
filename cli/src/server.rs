use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use engine::core::units::Bytes;
use engine::systems::launch::{self, ReadyLine, SERVER_ENV_PREFIX, ServerConfig, ServerReady};

pub const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A game server process, stopped when dropped.
pub struct GameServer {
    process: Child,
    ready: ServerReady,
}

impl GameServer {
    pub fn start(binary: &Path, config: &ServerConfig) -> GameServer {
        let mut process = Command::new(binary)
            .envs(launch::env_of(SERVER_ENV_PREFIX, config))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("{} doesn't run: {error}", binary.display()));
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
        let ready = readied
            .recv_timeout(READY_WITHIN)
            .unwrap_or_else(|_| panic!("{} never got ready", binary.display()));
        GameServer { process, ready }
    }

    pub fn id(&self) -> u32 {
        self.process.id()
    }

    pub fn http_url(&self) -> String {
        format!("http://{LOCALHOST}:{}", self.ready.http)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{LOCALHOST}:{}", self.ready.ws)
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
