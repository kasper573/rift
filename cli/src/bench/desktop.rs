//! A desktop of the bench's own for the client to play on: a headless screen, so the frame rate is
//! the client's alone, never held to the refresh of the machine's display or shared with its other
//! windows, and a sound sink no one hears, so the machine's speakers stay quiet.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub struct BenchDesktop {
    display: String,
    sink: String,
    _x: XServer,
    _compositor: Compositor,
    _speakers: Speakers,
}

impl BenchDesktop {
    pub fn open(folder: &Path) -> BenchDesktop {
        let name = format!(
            "rift-bench-{}-{}",
            std::process::id(),
            DESKTOPS.fetch_add(1, Ordering::Relaxed)
        );
        let process = Command::new("weston")
            .args([
                "--backend=headless",
                "--renderer=gl",
                "--shell=kiosk",
                "--no-config",
                // Nothing ever touches weston's own input: idle, it would blank the screen.
                "--idle-time=0",
            ])
            .arg(format!("--width={}", SCREEN.0))
            .arg(format!("--height={}", SCREEN.1))
            .arg(format!("--socket={name}"))
            .arg(format!("--log={}", folder.join("compositor.log").display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("weston runs: the bench plays the client on a headless screen of its own");
        let mut compositor = Compositor {
            process,
            socket: wayland_socket(&name),
        };
        compositor.await_ready(folder);
        let mut x = XServer(
            Command::new("Xwayland")
                // An X server resets once its last client leaves, and weston's kiosk shell crashes
                // on the rootful window that reset tears down.
                .args([
                    "-noreset",
                    "-displayfd",
                    "1",
                    "-nolisten",
                    "tcp",
                    "-geometry",
                ])
                .arg(format!("{}x{}", SCREEN.0, SCREEN.1))
                .env("WAYLAND_DISPLAY", &name)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("Xwayland runs: the client draws through X onto the headless screen"),
        );
        let mut number = String::new();
        let named = BufReader::new(x.0.stdout.take().expect("a piped stdout"))
            .read_line(&mut number)
            .expect("Xwayland names its display");
        assert!(named > 0, "Xwayland exited before naming its display");
        let speakers = Speakers::plug(&name);
        BenchDesktop {
            display: format!(":{}", number.trim()),
            sink: name,
            _x: x,
            _compositor: compositor,
            _speakers: speakers,
        }
    }

    /// Fits the window of the client `pid` to the screen and gives it the keyboard focus, as a window
    /// manager would: an unfocused window draws at a fraction of the frames.
    pub fn seat(&self, pid: u32) {
        let (pid, width, height) = (pid.to_string(), SCREEN.0.to_string(), SCREEN.1.to_string());
        let seated = Command::new("xdotool")
            .args([
                "search",
                "--sync",
                "--onlyvisible",
                "--pid",
                &pid,
                "--name",
                ".",
            ])
            .args(["windowmove", "--sync", "0", "0"])
            .args(["windowsize", "--sync", &width, &height])
            .args(["windowfocus", "--sync"])
            .env("DISPLAY", &self.display)
            .status()
            .expect("xdotool runs: the bench seats the client's window on its screen");
        assert!(
            seated.success(),
            "xdotool could not seat the client's window"
        );
    }

    /// The environment that puts a client on this desktop.
    pub fn env(&self) -> [(&str, &str); 3] {
        [
            ("DISPLAY", &self.display),
            ("PULSE_SINK", &self.sink),
            // Under PipeWire the client plays through its ALSA plugin, which reads only
            // PIPEWIRE_NODE; PULSE_SINK alone would leave it on the machine's speakers.
            ("PIPEWIRE_NODE", &self.sink),
        ]
    }
}

const SCREEN: (u32, u32) = (1280, 720);
const READY_WITHIN: Duration = Duration::from_secs(20);

static DESKTOPS: AtomicUsize = AtomicUsize::new(0);

struct XServer(Child);

impl Drop for XServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Compositor {
    process: Child,
    socket: PathBuf,
}

impl Compositor {
    fn await_ready(&mut self, folder: &Path) {
        let since = Instant::now();
        while !self.socket.exists() {
            if let Ok(Some(status)) = self.process.try_wait() {
                panic!(
                    "weston exited with {status} before it was ready: see {}",
                    folder.join("compositor.log").display()
                );
            }
            assert!(since.elapsed() < READY_WITHIN, "weston never got ready");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// Killed, weston leaves its socket and lock behind.
impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(self.socket.with_extension("lock"));
    }
}

struct Speakers {
    module: String,
}

impl Speakers {
    fn plug(sink: &str) -> Speakers {
        let loaded = Command::new("pactl")
            .args(["load-module", "module-null-sink"])
            .arg(format!("sink_name={sink}"))
            .output()
            .expect("pactl runs: the bench gives the client a sink of its own on the sound server");
        assert!(
            loaded.status.success(),
            "pactl could not add a sink for {sink}"
        );
        Speakers {
            module: String::from_utf8_lossy(&loaded.stdout).trim().to_owned(),
        }
    }
}

// Unplugged when dropped, so a bench that fails leaves no sink behind on the sound server.
impl Drop for Speakers {
    fn drop(&mut self) {
        let _ = Command::new("pactl")
            .args(["unload-module", &self.module])
            .status();
    }
}

fn wayland_socket(name: &str) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .expect("XDG_RUNTIME_DIR, where weston opens its socket");
    Path::new(&runtime).join(name)
}
