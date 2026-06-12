//! An honest gameplay session, end to end: the shipped `rift` binary runs against a freshly spawned
//! server on a real display, receives genuine OS mouse/keyboard input, and every assertion reads the
//! pixels a player would see — never the game's internals.
//!
//! The client and server are located by `RIFT_E2E_CLIENT` / `RIFT_E2E_SERVER` (CI points them at the
//! release builds), so this crate never compiles them. It is `#[ignore]`d: it needs a display and is
//! slow, so it runs only in CI (`cargo test -p e2e -- --ignored`), never in the default `cargo test`.
//!
//! The display layer differs by OS — Xvfb + x11rb on Linux, the runner's desktop + xcap elsewhere —
//! but the gameplay assertions are identical everywhere.

use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use enigo::{Direction, Enigo, Key, Keyboard};

const ARTIFACTS: &str = env!("CARGO_TARGET_TMPDIR");
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

/// The world is on screen once the view's center shows real scenery: tens of distinct colors even in
/// pixel art's tight palette, against the single flat clear color drawn there before (the HUD sits in
/// the corners).
const SCENE_COLORS: usize = 16;
/// Walking scrolls the whole camera. Tiled scenery is self-similar, so a scroll changes ~40% of the
/// pixels; idle animation changes only a few percent.
const WALKED: f64 = 0.2;

#[test]
#[ignore = "e2e: needs a display and is slow; CI-only, run with `cargo test -p e2e -- --ignored`"]
fn a_player_joins_and_visibly_walks() {
    let display = Display::start();
    let server = GameServer::start();
    let _client = display.spawn_client(&server.url);
    let mut enigo = Enigo::new(&display.input_settings()).expect("start OS input");

    let window = display.wait_for_window(Duration::from_secs(120));
    let before = wait_for_scene(&window, Duration::from_secs(120));
    save(&before, "before");

    // Click mid-view in each direction until one walks the player (and scrolls the camera with it);
    // a tap of space first revives a player that spawn-side enemies managed to kill, since any key
    // respawns the dead and an unbound key does nothing to the living.
    let (width, height) = (before.width as i32, before.height as i32);
    let mut moved = 0.0;
    for (dx, dy) in [(200, 0), (0, 150), (-200, 0), (0, -150)] {
        tap_space(&mut enigo);
        window.click(&mut enigo, width / 2 + dx, height / 2 + dy);
        sleep(Duration::from_secs(2));
        let after = window.capture();
        moved = diff_fraction(&before, &after);
        save(&after, "after");
        println!(
            "clicked ({dx:+}, {dy:+}): {:.0}% of pixels changed",
            moved * 100.0
        );
        if moved > WALKED {
            break;
        }
    }
    assert!(
        moved > WALKED,
        "clicking must visibly walk the player ({:.0}% of pixels changed, needed more than {:.0}%; \
         see {ARTIFACTS}/*.png)",
        moved * 100.0,
        WALKED * 100.0,
    );
}

fn wait_for_scene(window: &Window, timeout: Duration) -> Image {
    let deadline = Instant::now() + timeout;
    loop {
        let image = window.capture();
        let colors = distinct_colors(&center(&image));
        if colors > SCENE_COLORS {
            println!("the world is on screen ({colors} distinct colors mid-view)");
            sleep(Duration::from_millis(300));
            return window.capture();
        }
        if Instant::now() >= deadline {
            save(&image, "timeout");
            panic!(
                "the world never appeared: {colors} distinct colors mid-view, a scene shows more \
                 than {SCENE_COLORS} (see {ARTIFACTS}/timeout.png and client.err)"
            );
        }
        sleep(Duration::from_millis(500));
    }
}

fn tap_space(enigo: &mut Enigo) {
    enigo.key(Key::Space, Direction::Click).expect("tap space");
    sleep(Duration::from_millis(100));
}

struct GameServer {
    url: String,
    _proc: Proc,
}

impl GameServer {
    fn start() -> GameServer {
        let port = 30000 + (std::process::id() % 20000) as u16;
        let url = format!("http://127.0.0.1:{port}");
        let mut command = Command::new(server_bin());
        command
            .env("RIFT_ASSETS", ASSETS)
            .env("RIFT_GAME_SERVER_AUTH_BYPASS", "true")
            .env("RIFT_GAME_SERVER_PORT", port.to_string())
            .env_remove("RIFT_GAME_SERVER_PUBLIC_ADDR")
            .env_remove("RIFT_AUTH_ISSUER")
            .env_remove("RIFT_AUTH_AUDIENCE")
            .env_remove("RIFT_AUTH_JWKS_URI");
        let proc = Proc::start(command, "server");

        let deadline = Instant::now() + Duration::from_secs(30);
        let health = format!("{url}/health");
        loop {
            if ureq::get(&health).call().is_ok() {
                return GameServer { url, _proc: proc };
            }
            assert!(Instant::now() < deadline, "the server never became healthy");
            sleep(Duration::from_millis(200));
        }
    }
}

/// The shipped client binary; CI points this at the release build so the test exercises the exact
/// bytes that ship.
fn client_bin() -> PathBuf {
    bin("RIFT_E2E_CLIENT")
}

fn server_bin() -> PathBuf {
    bin("RIFT_E2E_SERVER")
}

fn bin(var: &str) -> PathBuf {
    let path = std::env::var_os(var)
        .unwrap_or_else(|| panic!("set {var} to the binary to test (CI builds it; see Justfile)"));
    PathBuf::from(path)
}

/// A child process killed on drop, with its output captured under the test artifacts.
struct Proc(Child);

impl Proc {
    fn start(mut command: Command, name: &str) -> Proc {
        let log = |extension| {
            File::create(artifacts().join(format!("{name}.{extension}"))).expect("create log")
        };
        let child = command
            .stdout(Stdio::from(log("out")))
            .stderr(Stdio::from(log("err")))
            .spawn()
            .unwrap_or_else(|error| panic!("could not start {name}: {error}"));
        Proc(child)
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A captured frame in row-major RGB.
struct Image {
    width: u16,
    height: u16,
    rgb: Vec<u8>,
}

fn distinct_colors(image: &Image) -> usize {
    image
        .rgb
        .chunks_exact(3)
        .map(|px| [px[0], px[1], px[2]])
        .collect::<HashSet<_>>()
        .len()
}

/// The middle half of the frame, where only the world is ever drawn.
fn center(image: &Image) -> Image {
    let (width, height) = (image.width / 2, image.height / 2);
    let (left, top) = (image.width / 4, image.height / 4);
    let mut rgb = Vec::with_capacity(3 * width as usize * height as usize);
    for y in top..top + height {
        let row = 3 * (y as usize * image.width as usize + left as usize);
        rgb.extend_from_slice(&image.rgb[row..row + 3 * width as usize]);
    }
    Image { width, height, rgb }
}

/// The fraction of pixels that visibly differ between two same-size frames.
fn diff_fraction(a: &Image, b: &Image) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height), "frame sizes");
    let changed = a
        .rgb
        .chunks_exact(3)
        .zip(b.rgb.chunks_exact(3))
        .filter(|(a, b)| {
            let delta = |i: usize| (a[i] as i32 - b[i] as i32).unsigned_abs();
            delta(0) + delta(1) + delta(2) > 24
        })
        .count();
    changed as f64 / (a.rgb.len() / 3) as f64
}

fn save(image: &Image, name: &str) {
    let file = File::create(artifacts().join(format!("{name}.png"))).expect("create png");
    let mut encoder = png::Encoder::new(file, image.width.into(), image.height.into());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("png header")
        .write_image_data(&image.rgb)
        .expect("png data");
}

fn artifacts() -> PathBuf {
    let dir = PathBuf::from(ARTIFACTS);
    std::fs::create_dir_all(&dir).expect("artifacts dir");
    dir
}

// The window title the client sets; the handle xcap finds it by where a window manager runs.
#[cfg(not(target_os = "linux"))]
const WINDOW_TITLE: &str = "rift mmo";

#[cfg(target_os = "linux")]
mod platform {
    //! Linux CI is headless: the test brings up its own Xvfb display, reads pixels off it with
    //! x11rb's `GetImage`, and walks the root window tree to find the client (no compositor sets the
    //! EWMH client list xcap would need).

    use super::{Command, Duration, Image, Instant, Proc, artifacts, sleep};
    use std::path::Path;

    use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ConnectionExt, ImageFormat, InputFocus, MapState, Window as XWindow,
    };
    use x11rb::rust_connection::RustConnection;

    pub struct Display {
        name: String,
        _proc: Proc,
    }

    impl Display {
        pub fn start() -> Display {
            let mut number = 100 + std::process::id() % 9000;
            while Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
                number += 1;
            }
            let name = format!(":{number}");
            let mut command = Command::new("Xvfb");
            command.args([&name, "-screen", "0", "1280x960x24"]);
            let proc = Proc::start(command, "xvfb");

            let socket = format!("/tmp/.X11-unix/X{number}");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !Path::new(&socket).exists() {
                assert!(
                    Instant::now() < deadline,
                    "Xvfb never came up on {name} — is Xvfb installed?"
                );
                sleep(Duration::from_millis(100));
            }
            Display { name, _proc: proc }
        }

        pub fn input_settings(&self) -> Settings {
            Settings {
                x11_display: Some(self.name.clone()),
                open_prompt_to_get_permissions: false,
                ..Settings::default()
            }
        }

        pub fn spawn_client(&self, game_url: &str) -> Proc {
            let mut command = Command::new(super::client_bin());
            command
                .env("DISPLAY", &self.name)
                // Wayland would win over the private X display and open on the developer's screen.
                .env_remove("WAYLAND_DISPLAY");
            super::client_env(&mut command, game_url);
            Proc::start(command, "client")
        }

        pub fn wait_for_window(&self, timeout: Duration) -> Window {
            let (conn, screen) = x11rb::connect(Some(&self.name)).expect("connect to Xvfb");
            let root = conn.setup().roots[screen].root;
            let deadline = Instant::now() + timeout;
            loop {
                let children = conn
                    .query_tree(root)
                    .expect("query tree")
                    .reply()
                    .expect("tree reply")
                    .children;
                for window in children {
                    let viewable = conn
                        .get_window_attributes(window)
                        .expect("window attributes")
                        .reply()
                        .is_ok_and(|reply| reply.map_state == MapState::VIEWABLE);
                    if viewable && geometry(&conn, window).0 >= 1024 {
                        // Nothing focuses windows on a bare Xvfb, and key events go to the focus
                        // owner; without this, keyboard input would silently vanish.
                        conn.set_input_focus(InputFocus::POINTER_ROOT, window, x11rb::CURRENT_TIME)
                            .expect("set focus")
                            .check()
                            .expect("focus reply");
                        return Window {
                            conn,
                            root,
                            id: window,
                        };
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "the client never opened its window (see {}/client.err)",
                    artifacts().display()
                );
                sleep(Duration::from_millis(250));
            }
        }
    }

    pub struct Window {
        conn: RustConnection,
        root: XWindow,
        id: XWindow,
    }

    impl Window {
        pub fn click(&self, enigo: &mut Enigo, x: i32, y: i32) {
            let at = self
                .conn
                .translate_coordinates(self.id, self.root, x as i16, y as i16)
                .expect("translate coordinates")
                .reply()
                .expect("translate reply");
            enigo
                .move_mouse(at.dst_x.into(), at.dst_y.into(), Coordinate::Abs)
                .expect("move pointer");
            sleep(Duration::from_millis(100));
            enigo.button(Button::Left, Direction::Click).expect("click");
        }

        pub fn capture(&self) -> Image {
            let (width, height) = geometry(&self.conn, self.id);
            let reply = self
                .conn
                .get_image(ImageFormat::Z_PIXMAP, self.id, 0, 0, width, height, !0)
                .expect("get image")
                .reply()
                .expect("image reply");
            let rgb = reply
                .data
                .chunks_exact(4)
                .flat_map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]])
                .collect();
            Image { width, height, rgb }
        }
    }

    fn geometry(conn: &RustConnection, window: XWindow) -> (u16, u16) {
        let reply = conn
            .get_geometry(window)
            .expect("get geometry")
            .reply()
            .expect("geometry reply");
        (reply.width, reply.height)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    //! Windows and macOS runners have a real desktop: the client opens a normal window there, xcap
    //! finds it by title and captures it, and enigo drives the system pointer and keyboard.

    use super::{Command, Duration, Image, Instant, Proc, WINDOW_TITLE, sleep};

    use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
    use xcap::Window as XcapWindow;

    pub struct Display;

    impl Display {
        pub fn start() -> Display {
            Display
        }

        pub fn input_settings(&self) -> Settings {
            Settings {
                open_prompt_to_get_permissions: false,
                ..Settings::default()
            }
        }

        pub fn spawn_client(&self, game_url: &str) -> Proc {
            let mut command = Command::new(super::client_bin());
            super::client_env(&mut command, game_url);
            Proc::start(command, "client")
        }

        pub fn wait_for_window(&self, timeout: Duration) -> Window {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(window) = find() {
                    if window.width().expect("width") >= 1024 {
                        return Window {
                            id: window.id().expect("window id"),
                        };
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "the client never opened its window (see client.err)"
                );
                sleep(Duration::from_millis(250));
            }
        }
    }

    pub struct Window {
        id: u32,
    }

    impl Window {
        fn current(&self) -> XcapWindow {
            XcapWindow::all()
                .expect("list windows")
                .into_iter()
                .find(|window| window.id().ok() == Some(self.id))
                .expect("the client window is gone")
        }

        pub fn click(&self, enigo: &mut Enigo, x: i32, y: i32) {
            let window = self.current();
            let (ox, oy) = (window.x().expect("x"), window.y().expect("y"));
            enigo
                .move_mouse(ox + x, oy + y, Coordinate::Abs)
                .expect("move pointer");
            sleep(Duration::from_millis(100));
            enigo.button(Button::Left, Direction::Click).expect("click");
        }

        pub fn capture(&self) -> Image {
            let image = self.current().capture_image().expect("capture window");
            let (width, height) = (image.width() as u16, image.height() as u16);
            let rgb = image
                .into_vec()
                .chunks_exact(4)
                .flat_map(|rgba| [rgba[0], rgba[1], rgba[2]])
                .collect();
            Image { width, height, rgb }
        }
    }

    fn find() -> Option<XcapWindow> {
        XcapWindow::all().ok()?.into_iter().find(|window| {
            window
                .title()
                .map(|title| title.to_lowercase().contains(WINDOW_TITLE))
                .unwrap_or(false)
        })
    }
}

use platform::{Display, Window};

/// The client env every platform shares: bypass SSO, point at the test server, and keep its config
/// out of the developer's real settings.
fn client_env(command: &mut Command, game_url: &str) {
    command
        .env_remove("RIFT_CLIENT_ISSUER")
        .env("RIFT_CLIENT_AUTH_BYPASS", "tester")
        .env("RIFT_CLIENT_GAME_URL", game_url)
        .env("RIFT_ASSETS", ASSETS)
        .env("XDG_CONFIG_HOME", artifacts().join("config"));
}
