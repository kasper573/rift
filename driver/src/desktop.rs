use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use engine::core::math::{Pos, WindowPx};
use engine::systems::area::AreaId;
use engine::systems::launch::{self, AuthMode, CLIENT_ENV_PREFIX, ClientConfig};
use engine::systems::probe::{ProbeInputSeen, ProbeSnapshot, ProbeTerrain};

use crate::content::{from_json, to_json};
use crate::memory::Bytes;
use crate::recording::{Footage, Recording};
use crate::seen::area_name;
use crate::server::Server;

/// The machine a desktop client plays on: a screen and speakers of its own, recorded, and a probe
/// to ask the client what it shows. Input goes in through the X server, as a player's would, and
/// every input waits until the game has received it, so nothing is lost or merged however slowly
/// frames come, and nothing waits longer than the game takes.
pub struct Desktop {
    client: Child,
    questions: TcpStream,
    answers: BufReader<TcpStream>,
    screen: Screen,
    _speakers: Speakers,
    recording: Option<Recording>,
    observer: Option<Observer>,
    folder: PathBuf,
    seen: ProbeInputSeen,
    cursor: Option<Pos<WindowPx>>,
    frame: u32,
    /// The ground of the area the client is in, which it sends only on arriving there.
    terrain: Option<(AreaId, ProbeTerrain)>,
    steps: File,
}

type Observer = Box<dyn FnMut(&ProbeSnapshot)>;

/// Waits give up only once both run out, so a slow machine gets the frames a fast one does.
#[derive(Clone, Copy)]
pub struct WaitBudget {
    pub time: Duration,
    pub frames: u32,
}

pub const BUDGET: WaitBudget = WaitBudget {
    time: Duration::from_secs(20),
    frames: 1200,
};

pub const SCREEN: (u32, u32) = (1280, 720);

#[derive(Clone, Copy)]
pub enum Button {
    Left = 1,
    Middle = 2,
    Right = 3,
}

impl Desktop {
    pub fn memory(&self) -> Bytes {
        crate::memory::resident(self.client.id())
    }

    pub fn start(
        server: &Server,
        binary: &Path,
        token: Option<String>,
        folder: &Path,
        recorded: bool,
    ) -> Desktop {
        std::fs::create_dir_all(folder).expect("a folder for the desktop");
        let screen = Screen::open(folder, SCREEN);
        let speakers = Speakers::plug();
        let recording = recorded
            .then(|| Recording::start(&screen.display, SCREEN, &speakers.monitor(), folder));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a probe port");
        let config = ClientConfig {
            game_server_url: server.http_url(),
            game_server_ws_url: server.ws_url(),
            auth: AuthMode::Fake,
            access_token: token,
            profile_dir: folder.join("profile"),
            exit_on_stdin_eof: true,
            probe: Some(listener.local_addr().expect("a bound probe port")),
        };
        let log = File::create(folder.join("client.log")).expect("a client log");
        let mut client = Command::new(binary)
            .envs(launch::env_of(CLIENT_ENV_PREFIX, &config))
            .env("DISPLAY", &screen.display)
            // Under PipeWire the client plays through its ALSA plugin, which reads only
            // PIPEWIRE_NODE; PULSE_SINK alone would leave it on the machine's speakers.
            .env("PULSE_SINK", &speakers.sink)
            .env("PIPEWIRE_NODE", &speakers.sink)
            .stdin(Stdio::piped())
            .stdout(log.try_clone().expect("a second handle on the client log"))
            .stderr(log)
            .spawn()
            .expect("the desktop client binary runs");
        let questions = accept(&listener, &mut client);
        questions
            .set_read_timeout(Some(STALL))
            .expect("a probe read timeout");
        let answers = BufReader::new(questions.try_clone().expect("a second handle on the probe"));
        let desktop = Desktop {
            client,
            questions,
            answers,
            screen,
            _speakers: speakers,
            recording,
            observer: None,
            folder: folder.to_path_buf(),
            seen: ProbeInputSeen::default(),
            cursor: None,
            frame: 0,
            terrain: None,
            steps: File::create(folder.join("steps.log")).expect("a steps log"),
        };
        // Nothing fits windows to the screen or hands out keyboard focus without a window manager.
        let pid = desktop.client.id().to_string();
        let (width, height) = (SCREEN.0.to_string(), SCREEN.1.to_string());
        desktop.xdotool(&[
            "search",
            "--sync",
            "--onlyvisible",
            "--pid",
            &pid,
            ".",
            "windowmove",
            "--sync",
            "0",
            "0",
            "windowsize",
            "--sync",
            &width,
            &height,
            "windowfocus",
            "--sync",
        ]);
        desktop
    }

    /// What the client shows in a frame after asking.
    pub fn snapshot(&mut self) -> ProbeSnapshot {
        writeln!(self.questions).expect("the client takes questions");
        let mut line = String::new();
        match self.answers.read_line(&mut line) {
            Ok(0) => panic!("the client hung up"),
            Ok(_) => {}
            Err(error) => panic!("the client stopped answering: {error}"),
        }
        let mut snapshot: ProbeSnapshot = from_json(&line);
        match (&snapshot.terrain, &self.terrain) {
            (Some(terrain), _) => {
                self.terrain = snapshot.area.map(|area| (area, terrain.clone()));
            }
            (None, Some((area, terrain))) if snapshot.area == Some(*area) => {
                snapshot.terrain = Some(terrain.clone());
            }
            _ => {}
        }
        self.seen = snapshot.input.seen;
        self.cursor = snapshot.input.cursor;
        self.frame = snapshot.frame;
        if let Some(observer) = self.observer.as_mut() {
            observer(&snapshot);
        }
        snapshot
    }

    pub fn cursor(&self) -> Option<Pos<WindowPx>> {
        self.cursor
    }

    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// Sees every frame the driver asks for.
    pub fn observe(&mut self, observer: impl FnMut(&ProbeSnapshot) + 'static) {
        self.observer = Some(Box::new(observer));
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn move_pointer(&mut self, to: Pos<WindowPx>) -> ProbeSnapshot {
        let to = to.round();
        self.xdotool(&["mousemove", &to.x.to_string(), &to.y.to_string()]);
        let seen = self.wait_for("the pointer to arrive", BUDGET, |snapshot| {
            let cursor = snapshot.input.cursor?;
            (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
        });
        self.note(&seen, &format!("pointer at {:?}", (to.x, to.y)));
        seen
    }

    /// Through every point in turn as fast as a flick of the hand, many to a frame, coming to rest
    /// on the last.
    pub fn sweep_pointer(&mut self, through: &[Pos<WindowPx>]) -> ProbeSnapshot {
        let to = through.last().expect("a point to sweep to").round();
        let moves: Vec<String> = through
            .iter()
            .flat_map(|at| {
                let at = at.round();
                [
                    "mousemove".to_owned(),
                    at.x.to_string(),
                    at.y.to_string(),
                    "sleep".to_owned(),
                    SWEEP_STEP.to_owned(),
                ]
            })
            .collect();
        self.xdotool(&moves.iter().map(String::as_str).collect::<Vec<_>>());
        let seen = self.wait_for("the pointer to come to rest", BUDGET, |snapshot| {
            let cursor = snapshot.input.cursor?;
            (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
        });
        self.note(&seen, &format!("pointer swept to {:?}", (to.x, to.y)));
        seen
    }

    /// Both clicks of a double click go out together: the game counts it by the time between them.
    /// Frames far enough apart can still split them, and then, as a player would, it clicks again.
    /// Each try waits out the run of clicks before it, which the game would otherwise count it on
    /// from.
    pub fn click(&mut self, button: Button, times: u8) -> ProbeSnapshot {
        let deadline = Deadline::from(self, BUDGET);
        loop {
            if times > 1 {
                self.wait_for("the click run before to close", BUDGET, |snapshot| {
                    (!snapshot.input.click_run_open).then_some(())
                });
            }
            let landed = self.click_once(button, times);
            if times == 1 || landed.input.seen.click_run == times {
                return landed;
            }
            if deadline.passed(self) {
                self.give_up(&format!("the game to take {times} clicks as one"));
            }
        }
    }

    pub fn button_down(&mut self, button: Button) -> ProbeSnapshot {
        let before = self.seen;
        self.xdotool(&["mousedown", &(button as u8).to_string()]);
        self.wait_for("the button to go down", BUDGET, |snapshot| {
            (snapshot.input.seen.mouse_presses > before.mouse_presses).then(|| snapshot.clone())
        })
    }

    pub fn button_up(&mut self, button: Button) -> ProbeSnapshot {
        let before = self.seen;
        self.xdotool(&["mouseup", &(button as u8).to_string()]);
        self.wait_for("the button to come up", BUDGET, |snapshot| {
            (snapshot.input.seen.mouse_releases > before.mouse_releases).then(|| snapshot.clone())
        })
    }

    /// X turns the wheel by clicking buttons 4 (up) and 5 (down), a line per click.
    pub fn wheel(&mut self, lines: i32) -> ProbeSnapshot {
        let before = self.seen;
        let button = if lines < 0 { "4" } else { "5" };
        let repeat = lines.unsigned_abs().to_string();
        self.xdotool(&["click", "--repeat", &repeat, "--delay", "10", button]);
        let turned = self.wait_for("the wheel to turn", BUDGET, |snapshot| {
            (snapshot.input.seen.wheel_turns >= before.wheel_turns + lines.unsigned_abs())
                .then(|| snapshot.clone())
        });
        self.note(&turned, &format!("wheel {lines} lines"));
        turned
    }

    pub fn key(&mut self, keysym: &str) -> ProbeSnapshot {
        let before = self.seen;
        self.xdotool(&["key", keysym]);
        let landed = self.wait_for("the key press to land", BUDGET, |snapshot| {
            (snapshot.input.seen.key_presses > before.key_presses).then(|| snapshot.clone())
        });
        self.note(&landed, &format!("key {keysym}"));
        landed
    }

    pub fn key_down(&mut self, keysym: &str) -> ProbeSnapshot {
        let before = self.seen;
        self.xdotool(&["keydown", keysym]);
        self.wait_for("the key to go down", BUDGET, |snapshot| {
            (snapshot.input.seen.key_presses > before.key_presses).then(|| snapshot.clone())
        })
    }

    /// Releases count nowhere, but X delivers them before anything sent after.
    pub fn key_up(&mut self, keysym: &str) {
        self.xdotool(&["keyup", keysym]);
    }

    /// Typed keys queue up in order, so they go out together; `landed` says when the game has them
    /// all.
    pub fn type_text(
        &mut self,
        text: &str,
        delay: Duration,
        landed: impl FnMut(&ProbeSnapshot) -> bool,
    ) -> ProbeSnapshot {
        let delay = delay.as_millis().to_string();
        self.xdotool(&["type", "--delay", &delay, "--", text]);
        let mut landed = landed;
        let seen = self.wait_for(&format!("{text:?} to reach the game"), BUDGET, |snapshot| {
            landed(snapshot).then(|| snapshot.clone())
        });
        self.note(&seen, &format!("typed {text:?}"));
        seen
    }

    pub fn wait_for<T>(
        &mut self,
        what: &str,
        budget: WaitBudget,
        found: impl FnMut(&ProbeSnapshot) -> Option<T>,
    ) -> T {
        match self.try_wait_for(budget, found) {
            Some(done) => done,
            None => self.give_up(what),
        }
    }

    /// As `wait_for`, but running out is an answer rather than a failure.
    pub fn try_wait_for<T>(
        &mut self,
        budget: WaitBudget,
        mut found: impl FnMut(&ProbeSnapshot) -> Option<T>,
    ) -> Option<T> {
        let deadline = Deadline::from(self, budget);
        loop {
            if let Some(done) = found(&self.snapshot()) {
                return Some(done);
            }
            if deadline.passed(self) {
                return None;
            }
        }
    }

    pub fn give_up(&mut self, what: &str) -> ! {
        self.fail(&format!("gave up waiting for {what}"))
    }

    fn click_once(&mut self, button: Button, times: u8) -> ProbeSnapshot {
        let before = self.seen;
        let times_arg = times.to_string();
        self.xdotool(&[
            "click",
            "--repeat",
            &times_arg,
            "--delay",
            "40",
            &(button as u8).to_string(),
        ]);
        let landed = self.wait_for("the click to land", BUDGET, |snapshot| {
            let seen = snapshot.input.seen;
            (seen.mouse_presses >= before.mouse_presses + u32::from(times)
                && seen.mouse_releases >= before.mouse_releases + u32::from(times))
            .then(|| snapshot.clone())
        });
        self.note(&landed, &format!("click {times}× button {}", button as u8));
        landed
    }

    /// What the player saw last is left beside the steps that led there.
    pub fn fail(&mut self, why: &str) -> ! {
        let snapshot = self.snapshot();
        self.note(&snapshot, why);
        let _ = std::fs::write(self.folder.join("last-snapshot.json"), to_json(&snapshot));
        panic!("{why}");
    }

    pub fn note(&mut self, snapshot: &ProbeSnapshot, step: &str) {
        let me = snapshot
            .me
            .as_ref()
            .map(|me| format!("{:?} {:?} locked {}", me.at, me.action, me.locked));
        let _ = writeln!(
            self.steps,
            "frame {}: {step} | claim {:?} | area {:?} | me {me:?} | stage {:?}",
            snapshot.frame,
            snapshot.claim,
            area_name(snapshot),
            snapshot.stage.as_ref().map(|stage| &stage.node),
        );
    }

    /// Ends the client and its recording.
    pub fn stop(&mut self) -> Option<Footage> {
        let _ = self.client.kill();
        let _ = self.client.wait();
        self.recording.take()?.finish()
    }

    fn xdotool(&self, args: &[&str]) {
        let run = Command::new("xdotool")
            .env("DISPLAY", &self.screen.display)
            .args(args)
            .output()
            .expect("xdotool runs");
        assert!(
            run.status.success(),
            "xdotool {args:?}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

// A failed test keeps what its screen showed.
impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(footage) = self.stop() {
            if std::thread::panicking() {
                footage.render(&self.folder.join("recording.mp4"), None);
            }
            footage.discard();
        }
    }
}

pub(crate) struct Deadline {
    since: Instant,
    first: u32,
    budget: WaitBudget,
}

impl Deadline {
    pub(crate) fn from(desktop: &Desktop, budget: WaitBudget) -> Deadline {
        Deadline {
            since: Instant::now(),
            first: desktop.frame,
            budget,
        }
    }

    pub(crate) fn passed(&self, desktop: &Desktop) -> bool {
        self.since.elapsed() >= self.budget.time
            && desktop.frame.saturating_sub(self.first) >= self.budget.frames
    }
}

// Only a hung client goes this long without a frame: on a machine run into the ground, a frame can
// take the better part of a minute.
const STALL: Duration = Duration::from_secs(300);
// Seconds between the moves of a sweep: a mouse reports a few hundred times a second.
const SWEEP_STEP: &str = "0.004";

// A virtual X screen of the desktop's own: a rootful Xwayland on a headless Wayland compositor.
// Unlike on Xvfb, a GPU's driver hands it frames without copying each one through the CPU.
struct Screen {
    display: String,
    _x: ScreenServer,
    _compositor: ScreenServer,
}

// Ended when dropped, so a screen set up halfway, when the next step fails, leaves nothing running.
struct ScreenServer(Child);

impl Drop for ScreenServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Screen {
    fn open(folder: &Path, size: (u32, u32)) -> Screen {
        let name = format!(
            "rift-playtest-{}-{}",
            std::process::id(),
            SCREENS.fetch_add(1, Ordering::Relaxed)
        );
        let ready = SocketAddr::from_abstract_name(&name).expect("a short enough socket name");
        let notices = UnixDatagram::bind_addr(&ready).expect("a socket to hear the compositor on");
        let mut compositor = ScreenServer(
            Command::new("weston")
                .args([
                    "--backend=headless",
                    "--renderer=gl",
                    "--shell=kiosk",
                    "--no-config",
                    // Input goes to Xwayland, out of weston's sight: idle, it would blank the
                    // screen.
                    "--idle-time=0",
                    "--modules=systemd-notify.so",
                ])
                .arg(format!("--width={}", size.0))
                .arg(format!("--height={}", size.1))
                .arg(format!("--socket={name}"))
                .arg(format!("--log={}", folder.join("compositor.log").display()))
                .env("NOTIFY_SOCKET", format!("@{name}"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("weston runs"),
        );
        await_ready(&notices, &mut compositor.0, folder);
        let mut x = ScreenServer(
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
                .arg(format!("{}x{}", size.0, size.1))
                .env("WAYLAND_DISPLAY", &name)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("Xwayland runs"),
        );
        let mut number = String::new();
        let named = BufReader::new(x.0.stdout.take().expect("a piped stdout"))
            .read_line(&mut number)
            .expect("Xwayland names its display");
        assert!(named > 0, "Xwayland exited before naming its display");
        Screen {
            display: format!(":{}", number.trim()),
            _x: x,
            _compositor: compositor,
        }
    }
}

static SCREENS: AtomicUsize = AtomicUsize::new(0);

static SINKS: AtomicUsize = AtomicUsize::new(0);

// A sound server sink of the desktop's own, so its recording hears this client alone.
struct Speakers {
    sink: String,
    module: String,
}

impl Speakers {
    fn plug() -> Speakers {
        let sink = format!(
            "rift-playtest-{}-{}",
            std::process::id(),
            SINKS.fetch_add(1, Ordering::Relaxed)
        );
        let loaded = Command::new("pactl")
            .args(["load-module", "module-null-sink"])
            .arg(format!("sink_name={sink}"))
            .output()
            .expect("pactl runs: the driver records through a PulseAudio sound server");
        assert!(
            loaded.status.success(),
            "pactl could not add a sink for {sink}"
        );
        let module = String::from_utf8_lossy(&loaded.stdout).trim().to_owned();
        Speakers { sink, module }
    }

    fn monitor(&self) -> String {
        format!("{}.monitor", self.sink)
    }
}

// Unplugged when dropped, so a desktop that fails to start leaves no sink behind on the sound
// server.
impl Drop for Speakers {
    fn drop(&mut self) {
        let _ = Command::new("pactl")
            .args(["unload-module", &self.module])
            .status();
    }
}

// The compositor says it's ready the way it would tell systemd.
fn await_ready(notices: &UnixDatagram, compositor: &mut Child, folder: &Path) {
    notices
        .set_nonblocking(true)
        .expect("a nonblocking notice socket");
    let since = Instant::now();
    let mut notice = [0; 64];
    loop {
        match notices.recv(&mut notice) {
            Ok(_) => return,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => panic!("the compositor's notice socket failed: {error}"),
        }
        if let Ok(Some(status)) = compositor.try_wait() {
            panic!(
                "weston exited with {status} before it was ready: see {}",
                folder.join("compositor.log").display()
            );
        }
        assert!(since.elapsed() < STALL, "weston never got ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn accept(listener: &TcpListener, client: &mut Child) -> TcpStream {
    listener
        .set_nonblocking(true)
        .expect("a nonblocking probe port");
    let since = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).expect("a blocking probe");
                return stream;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => panic!("the probe port failed: {error}"),
        }
        if let Ok(Some(status)) = client.try_wait() {
            panic!("the desktop client exited with {status} before its probe connected");
        }
        assert!(
            since.elapsed() < STALL,
            "the desktop client's probe never connected"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
