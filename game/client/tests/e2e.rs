//! An honest gameplay session, end to end: the real `rift` binary runs against a freshly spawned
//! server on a private X display (Xvfb), receives genuine mouse input through the XTEST
//! extension, and every assertion reads the pixels a player would see — never the game's
//! internals. Linux-only by design, matching CI; it needs `Xvfb` on the PATH.

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt, ImageFormat, InputFocus,
    KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, MapState, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const ARTIFACTS: &str = env!("CARGO_TARGET_TMPDIR");

/// The world is on screen once the view's center shows real scenery: tens of distinct colors
/// even in pixel art's tight palette, against the single flat clear color drawn there before
/// (the HUD sits in the corners).
const SCENE_COLORS: usize = 16;
/// Walking scrolls the whole camera. Tiled scenery is self-similar, so a scroll changes ~40% of
/// the pixels; idle animation changes only a few percent.
const WALKED: f64 = 0.2;

#[test]
fn a_player_joins_and_visibly_walks() {
    let xvfb = Xvfb::start();
    let server = GameServer::start();
    let _client = client(&xvfb.display, &server.url);

    let x = X::connect(&xvfb.display);
    let window = x.wait_for_window(Duration::from_secs(90));

    let before = x.wait_for_scene(window, Duration::from_secs(120));
    save(&before, "before");

    // Click mid-view in each direction until one walks the player (and scrolls the camera with
    // it); a tap of space first revives a player that spawn-side enemies managed to kill, since
    // any key respawns the dead and an unbound key does nothing to the living.
    let (width, height) = (before.width as i16, before.height as i16);
    let mut moved = 0.0;
    for (dx, dy) in [(200, 0), (0, 150), (-200, 0), (0, -150)] {
        x.tap_space();
        x.click(window, width / 2 + dx, height / 2 + dy);
        sleep(Duration::from_secs(2));
        let after = x.capture(window);
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
        "clicking must visibly walk the player ({:.0}% of pixels changed, needed more than \
         {:.0}%; see {ARTIFACTS}/*.png)",
        moved * 100.0,
        WALKED * 100.0,
    );
}

/// The real client binary, playing on `display` with a bypass identity.
fn client(display: &str, game_url: &str) -> Proc {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rift"));
    command
        .env("DISPLAY", display)
        // Wayland would win over the private X display and open on the developer's screen.
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("RIFT_CLIENT_ISSUER")
        .env("RIFT_CLIENT_AUTH_BYPASS", "tester")
        .env("RIFT_CLIENT_GAME_URL", game_url)
        .env("RIFT_ASSETS", ASSETS)
        .env("XDG_CONFIG_HOME", artifacts().join("config"));
    Proc::start(command, "client")
}

struct GameServer {
    url: String,
    _proc: Proc,
}

impl GameServer {
    fn start() -> GameServer {
        let port = 30000 + (std::process::id() % 20000) as u16;
        let url = format!("http://127.0.0.1:{port}");
        let mut command = Command::new(sibling_binary("server"));
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

/// The sibling binary `name` next to the client binary, built if missing.
fn sibling_binary(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_rift"))
        .parent()
        .expect("target dir")
        .join(name);
    if !path.exists() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", name])
            .status()
            .expect("run cargo build");
        assert!(status.success(), "failed to build {name}");
    }
    path
}

struct Xvfb {
    display: String,
    _proc: Proc,
}

impl Xvfb {
    fn start() -> Xvfb {
        let mut number = 100 + std::process::id() % 9000;
        while Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
            number += 1;
        }
        let display = format!(":{number}");
        let mut command = Command::new("Xvfb");
        command.args([&display, "-screen", "0", "1280x960x24"]);
        let proc = Proc::start(command, "xvfb");

        let socket = format!("/tmp/.X11-unix/X{number}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !Path::new(&socket).exists() {
            assert!(
                Instant::now() < deadline,
                "Xvfb never came up on {display} — is Xvfb installed?"
            );
            sleep(Duration::from_millis(100));
        }
        Xvfb {
            display,
            _proc: proc,
        }
    }
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

struct X {
    conn: RustConnection,
    root: Window,
}

struct Image {
    width: u16,
    height: u16,
    rgb: Vec<u8>,
}

impl X {
    fn connect(display: &str) -> X {
        let (conn, screen) = x11rb::connect(Some(display)).expect("connect to Xvfb");
        let root = conn.setup().roots[screen].root;
        X { conn, root }
    }

    /// Waits for the client's top-level window to be mapped, and returns it.
    fn wait_for_window(&self, timeout: Duration) -> Window {
        let deadline = Instant::now() + timeout;
        loop {
            let children = self
                .conn
                .query_tree(self.root)
                .expect("query tree")
                .reply()
                .expect("tree reply")
                .children;
            for window in children {
                let viewable = self
                    .conn
                    .get_window_attributes(window)
                    .expect("window attributes")
                    .reply()
                    .is_ok_and(|reply| reply.map_state == MapState::VIEWABLE);
                if viewable && self.geometry(window).0 >= 1024 {
                    // Nothing focuses windows on a bare Xvfb, and XTEST key events go to the
                    // focus owner; without this, keyboard input would silently vanish.
                    self.conn
                        .set_input_focus(InputFocus::POINTER_ROOT, window, x11rb::CURRENT_TIME)
                        .expect("set focus")
                        .check()
                        .expect("focus reply");
                    return window;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the client never opened its window (see {ARTIFACTS}/client.err)"
            );
            sleep(Duration::from_millis(250));
        }
    }

    /// Captures frames until the world is on screen, then lets it settle and returns a frame.
    fn wait_for_scene(&self, window: Window, timeout: Duration) -> Image {
        let deadline = Instant::now() + timeout;
        loop {
            let image = self.capture(window);
            let colors = distinct_colors(&center(&image));
            if colors > SCENE_COLORS {
                println!("the world is on screen ({colors} distinct colors mid-view)");
                sleep(Duration::from_millis(300));
                return self.capture(window);
            }
            if Instant::now() >= deadline {
                save(&image, "timeout");
                panic!(
                    "the world never appeared: {colors} distinct colors mid-view, a scene shows \
                     more than {SCENE_COLORS} (see {ARTIFACTS}/timeout.png and client.err)"
                );
            }
            sleep(Duration::from_millis(500));
        }
    }

    /// A genuine left click at window coordinates, through XTEST.
    fn click(&self, window: Window, x: i16, y: i16) {
        let at = self
            .conn
            .translate_coordinates(window, self.root, x, y)
            .expect("translate coordinates")
            .reply()
            .expect("translate reply");
        self.input(MOTION_NOTIFY_EVENT, 0, at.dst_x, at.dst_y);
        self.conn.flush().expect("flush");
        sleep(Duration::from_millis(100));
        self.input(BUTTON_PRESS_EVENT, 1, 0, 0);
        self.input(BUTTON_RELEASE_EVENT, 1, 0, 0);
        self.conn.flush().expect("flush");
    }

    fn tap_space(&self) {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let mapping = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .expect("keyboard mapping")
            .reply()
            .expect("mapping reply");
        let space = mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .position(|syms| syms.contains(&u32::from(b' ')))
            .map(|index| min + index as u8)
            .expect("a space key exists");
        self.input(KEY_PRESS_EVENT, space, 0, 0);
        self.input(KEY_RELEASE_EVENT, space, 0, 0);
        self.conn.flush().expect("flush");
        sleep(Duration::from_millis(100));
    }

    fn input(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)
            .expect("fake input");
    }

    fn capture(&self, window: Window) -> Image {
        let (width, height) = self.geometry(window);
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, window, 0, 0, width, height, !0)
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

    fn geometry(&self, window: Window) -> (u16, u16) {
        let reply = self
            .conn
            .get_geometry(window)
            .expect("get geometry")
            .reply()
            .expect("geometry reply");
        (reply.width, reply.height)
    }
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
