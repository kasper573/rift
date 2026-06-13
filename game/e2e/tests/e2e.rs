//! A real gameplay session, end to end: the `rift` client binary runs against a freshly spawned
//! server on a real display, receives genuine OS mouse/keyboard input, and every assertion reads the
//! pixels a player would see — never the game's internals.
//!
//! Auth is the real thing too. The client opens the system browser on the stack's keycloak, the
//! test registers a fresh account in that browser window through the keyboard, and the session
//! continues with the token the realm minted — so sign-in, registration, token verification, and
//! session minting are all exercised. The docker stack must be up and its CA trusted (see the
//! README); run locally with `just e2e` (drives the desktop you're on).
//!
//! One path serves every OS: `enigo` injects input and `xcap` finds windows by title and captures
//! them; the display and browser underneath are the pipeline's to provide, so nothing here is
//! OS-specific.
//!
//! The client and server are located by `RIFT_E2E_CLIENT` / `RIFT_E2E_SERVER`, so this crate never
//! compiles them. It is `#[ignore]`d: it needs a display, a browser, and the stack, so it runs
//! only in CI (`cargo test -p e2e -- --ignored`) and via `just e2e`.

use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arboard::Clipboard;
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use xcap::Window;

const ARTIFACTS: &str = env!("CARGO_TARGET_TMPDIR");
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
// The title the client sets on its window; xcap finds it by this.
const WINDOW_TITLE: &str = "rift mmo";
// What the stack's realm pages title the browser window; same on the login and register pages.
const BROWSER_TITLE: &str = "sign in to rift";
const ISSUER: &str = "https://auth.rift.localhost/realms/rift";

/// The world is on screen once the mid-view fills with scenery rather than a flat screen behind a
/// label. Diced into a grid, the world makes most cells busy with several colors each; the sign-in,
/// failure, and mode screens are a flat fill with a few centered words, so only the handful of cells
/// crossing that text are busy — well under this fraction.
const SCENE_CELLS: f64 = 0.3;
/// Grid resolution over the mid-view, and the distinct-color count above which a cell reads as
/// scenery rather than flat fill (a stray anti-aliased text edge never reaches it).
const GRID: u16 = 8;
const CELL_COLORS: usize = 4;
/// Walking scrolls the whole camera. Tiled scenery is self-similar, so a scroll changes ~40% of the
/// pixels; idle animation changes only a few percent.
const WALKED: f64 = 0.2;

#[test]
#[ignore = "e2e: needs a display, a browser and the stack; CI-only, run with `just e2e`"]
fn a_player_registers_and_visibly_walks() {
    // Prod mode (RIFT_E2E_PROD) drives the released binary against the live deployment: no local
    // server, and the client keeps its baked-in prod endpoints and resolves assets beside the
    // executable. Otherwise a fresh local server is spawned and the client is pointed at it.
    let server = (!prod()).then(GameServer::start);
    let _client = spawn_client(server.as_ref().map(|server| server.url.as_str()));
    let mut enigo = Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    })
    .expect("start OS input");

    let game = wait_for_window(Duration::from_secs(120));
    register_in_browser(&mut enigo);

    // Raise the game window now that the browser is gone (X captures read the framebuffer, so an
    // occluded window would capture whatever covered it).
    game.click(&mut enigo, 20, 20);
    let before = wait_for_scene(&game, Duration::from_secs(120));
    save(&before, "before");

    // Click mid-view in each direction until one walks the player (and scrolls the camera with it);
    // a tap of space first revives a player that spawn-side enemies managed to kill, since any key
    // respawns the dead and an unbound key does nothing to the living.
    let (width, height) = (before.width as i32, before.height as i32);
    let mut moved = 0.0;
    for (dx, dy) in [(200, 0), (0, 150), (-200, 0), (0, -150)] {
        tap_space(&mut enigo);
        game.click(&mut enigo, width / 2 + dx, height / 2 + dy);
        sleep(Duration::from_secs(2));
        let after = game.capture();
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

/// Registers a fresh account in the browser window the client opened, driven purely through the
/// keyboard so no theme pixel positions are assumed.
fn register_in_browser(enigo: &mut Enigo) {
    // The realm's `registrations` endpoint takes the same OIDC parameters as `auth`, so swapping
    // the path continues the exact sign-in the client started — state, nonce, PKCE and redirect
    // included — and registering lands back on the client's loopback listener like a login would.
    let register_url = wait_for_authorize_url(Duration::from_secs(60)).replace(
        "/protocol/openid-connect/auth?",
        "/protocol/openid-connect/registrations?",
    );
    let browser = wait_for_browser(Duration::from_secs(60));
    // Paste the URL into the address bar rather than type it: a 40-character PKCE code_challenge
    // sent key-by-key loses characters on a loaded X server, and keycloak then rejects the request
    // as invalid. The clipboard transfers the whole URL atomically. `clipboard` is held to the end
    // of the function so it keeps serving the selection until chrome has pasted.
    let mut clipboard = Clipboard::new().expect("open clipboard");
    clipboard.set_text(&register_url).expect("set clipboard");
    browser.click(enigo, 100, 10);
    chord(enigo, Key::Control, Key::Unicode('l'));
    chord(enigo, Key::Control, Key::Unicode('v'));
    // A history match can inline-autocomplete a selected suffix; Delete drops it (no-op otherwise).
    tap(enigo, Key::Delete);
    tap(enigo, Key::Return);
    sleep(Duration::from_secs(8));

    // Keyboard-only form fill, calibrated to the pinned keycloak version and theme. Tab order on
    // its register page: username, password, [reveal], password-confirm, [reveal], email, submit.
    let stamp = unix_now().as_secs();
    let user = format!("tester{stamp}");
    let password = format!("e2e-{stamp}-pw");
    let fields: [(usize, &str); 4] = [
        (1, &user),
        (1, &password),
        (2, &password),
        (2, &format!("{user}@example.com")),
    ];
    for (tabs, value) in fields {
        for _ in 0..tabs {
            tap(enigo, Key::Tab);
        }
        // Paste each value too: typed key-by-key, a field can lose a character under load (a dropped
        // `@` makes the email invalid, a dropped password char fails the confirm match).
        clipboard.set_text(value).expect("set clipboard");
        chord(enigo, Key::Control, Key::Unicode('v'));
        sleep(Duration::from_millis(100));
    }
    tap(enigo, Key::Tab);
    tap(enigo, Key::Return);
    println!("registered {user}; waiting for the redirect to the client");
    sleep(Duration::from_secs(8));
    // Snapshot the page before closing it: if sign-in never completes, this shows whether
    // registration errored or a browser prompt got in the way.
    save(&browser.capture(), "register-result");

    // Close the browser so it cannot cover the game window; the redirect page is done with.
    chord(enigo, Key::Control, Key::Unicode('w'));
    sleep(Duration::from_secs(1));
}

/// The authorize URL the client prints when it opens the browser.
fn wait_for_authorize_url(timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        for log in ["client.out", "client.err"] {
            let content = std::fs::read_to_string(artifacts().join(log)).unwrap_or_default();
            if let Some(url) = content
                .split_whitespace()
                .find(|word| word.contains("/protocol/openid-connect/auth?"))
            {
                return url.to_owned();
            }
        }
        assert!(
            Instant::now() < deadline,
            "the client never printed its sign-in URL (see {}/client.*)",
            artifacts().display()
        );
        sleep(Duration::from_millis(250));
    }
}

fn wait_for_browser(timeout: Duration) -> Win {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = find_window(BROWSER_TITLE) {
            sleep(Duration::from_millis(500));
            return Win {
                id: window.id().expect("window id"),
            };
        }
        if Instant::now() >= deadline {
            let titles: Vec<String> = Window::all()
                .into_iter()
                .flatten()
                .filter_map(|w| w.title().ok())
                .collect();
            panic!(
                "no browser window showed the sign-in page; windows seen: {titles:?} (see {}/client.*)",
                artifacts().display()
            );
        }
        sleep(Duration::from_millis(250));
    }
}

/// The client window once it is on screen and full size; panics if it never appears.
fn wait_for_window(timeout: Duration) -> Win {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = find_window(WINDOW_TITLE)
            && window.width().unwrap_or(0) >= 1024
        {
            return Win {
                id: window.id().expect("window id"),
            };
        }
        assert!(
            Instant::now() < deadline,
            "the client never opened its window (see {}/client.err)",
            artifacts().display()
        );
        sleep(Duration::from_millis(250));
    }
}

fn find_window(title: &str) -> Option<Window> {
    Window::all().ok()?.into_iter().find(|window| {
        window
            .title()
            .map(|t| t.to_lowercase().contains(title))
            .unwrap_or(false)
    })
}

/// A handle to an on-screen window. xcap hands out window snapshots, so each call re-resolves the
/// live window by id.
struct Win {
    id: u32,
}

impl Win {
    fn window(&self) -> Window {
        Window::all()
            .expect("list windows")
            .into_iter()
            .find(|window| window.id().ok() == Some(self.id))
            .expect("the window is gone")
    }

    fn capture(&self) -> Image {
        let image = self.window().capture_image().expect("capture window");
        let (width, height) = (image.width() as u16, image.height() as u16);
        let rgb = image
            .into_vec()
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[0], rgba[1], rgba[2]])
            .collect();
        Image { width, height, rgb }
    }

    /// Clicks at a window-relative point by moving the system pointer to its absolute screen position.
    fn click(&self, enigo: &mut Enigo, x: i32, y: i32) {
        let window = self.window();
        let (origin_x, origin_y) = (window.x().expect("x"), window.y().expect("y"));
        enigo
            .move_mouse(origin_x + x, origin_y + y, Coordinate::Abs)
            .expect("move pointer");
        sleep(Duration::from_millis(100));
        enigo.button(Button::Left, Direction::Click).expect("click");
        sleep(Duration::from_millis(200));
    }
}

fn wait_for_scene(window: &Win, timeout: Duration) -> Image {
    let deadline = Instant::now() + timeout;
    loop {
        let image = window.capture();
        let scenery = scene_fraction(&center(&image));
        if scenery >= SCENE_CELLS {
            println!(
                "the world is on screen ({:.0}% of mid-view cells show scenery)",
                scenery * 100.0
            );
            sleep(Duration::from_millis(300));
            return window.capture();
        }
        if Instant::now() >= deadline {
            save(&image, "timeout");
            // A leftover realm page means registration never completed; its pixels say why.
            if let Some(browser) = find_window(BROWSER_TITLE) {
                let id = browser.id().expect("window id");
                save(&Win { id }.capture(), "browser");
            }
            panic!(
                "the world never appeared: {:.0}% of mid-view cells show scenery, a scene fills at \
                 least {:.0}% (see {ARTIFACTS}/timeout.png and client.err)",
                scenery * 100.0,
                SCENE_CELLS * 100.0,
            );
        }
        sleep(Duration::from_millis(500));
    }
}

fn tap(enigo: &mut Enigo, key: Key) {
    enigo.key(key, Direction::Click).expect("tap key");
    sleep(Duration::from_millis(100));
}

fn chord(enigo: &mut Enigo, modifier: Key, key: Key) {
    enigo.key(modifier, Direction::Press).expect("hold");
    sleep(Duration::from_millis(50));
    enigo.key(key, Direction::Click).expect("tap");
    sleep(Duration::from_millis(50));
    enigo.key(modifier, Direction::Release).expect("release");
    sleep(Duration::from_millis(200));
}

fn tap_space(enigo: &mut Enigo) {
    tap(enigo, Key::Space);
}

fn spawn_client(game_server_url: Option<&str>) -> Proc {
    // A browser inheriting this XDG_CONFIG_HOME gets a fresh profile; pre-marking chrome's first
    // run keeps its welcome dialog from covering the page.
    let config = artifacts().join("config");
    std::fs::create_dir_all(config.join("google-chrome")).expect("chrome config dir");
    let _ = File::create(config.join("google-chrome").join("First Run"));

    let mut command = Command::new(client_bin());
    command
        // Use the test's display, and never let a stray Wayland socket pull the window elsewhere.
        .env_remove("WAYLAND_DISPLAY")
        .env("XDG_CONFIG_HOME", config);
    // Local stack: point the client at the freshly spawned server and the test assets. Prod: the
    // released binary bakes in the prod endpoints and resolves assets beside the executable, so
    // override nothing.
    if let Some(url) = game_server_url {
        command
            .env("RIFT_CLIENT_ISSUER", ISSUER)
            .env("RIFT_CLIENT_GAME_SERVER_URL", url)
            .env("RIFT_ASSETS_DIR", ASSETS);
    }
    Proc::start(command, "client")
}

fn prod() -> bool {
    std::env::var_os("RIFT_E2E_PROD").is_some()
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
            .env("RIFT_ASSETS_DIR", ASSETS)
            .env("RIFT_GAME_SERVER_PORT", port.to_string())
            .env("RIFT_AUTH_ISSUER", ISSUER)
            .env("RIFT_AUTH_AUDIENCE", "rift")
            .env(
                "RIFT_AUTH_JWKS_URI",
                format!("{ISSUER}/protocol/openid-connect/certs"),
            )
            .env("RIFT_GAME_SERVER_PUBLIC_HOST", "127.0.0.1");
        let proc = Proc::start(command, "server");

        // Healthy only once it can verify tokens, so this also waits out the stack's keycloak.
        let deadline = Instant::now() + Duration::from_secs(60);
        let health = format!("{url}/health");
        loop {
            if ureq::get(&health).call().is_ok() {
                return GameServer { url, _proc: proc };
            }
            assert!(
                Instant::now() < deadline,
                "the server never became healthy (is the stack up? see `just stack`)"
            );
            sleep(Duration::from_millis(200));
        }
    }
}

/// The client binary under test — the same binary CI later releases, so the test asserts on what
/// ships.
fn client_bin() -> PathBuf {
    bin("RIFT_E2E_CLIENT")
}

fn server_bin() -> PathBuf {
    bin("RIFT_E2E_SERVER")
}

fn bin(var: &str) -> PathBuf {
    let path = std::env::var_os(var).unwrap_or_else(|| {
        panic!("set {var} to the binary to test (CI builds it; see `just e2e`)")
    });
    PathBuf::from(path)
}

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after the unix epoch")
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

/// The share of the frame's cells that carry scenery: diced into a `GRID`×`GRID` grid, the cells
/// showing more than `CELL_COLORS` distinct colors. A flat fill scores near zero; a few words of
/// text light only the cells they cross; tiled scenery lights most of them.
fn scene_fraction(image: &Image) -> f64 {
    let (cell_w, cell_h) = (image.width / GRID, image.height / GRID);
    if cell_w == 0 || cell_h == 0 {
        return 0.0;
    }
    let mut busy = 0;
    for gy in 0..GRID {
        for gx in 0..GRID {
            let mut colors = HashSet::new();
            for y in gy * cell_h..(gy + 1) * cell_h {
                for x in gx * cell_w..(gx + 1) * cell_w {
                    let i = 3 * (y as usize * image.width as usize + x as usize);
                    colors.insert([image.rgb[i], image.rgb[i + 1], image.rgb[i + 2]]);
                }
            }
            if colors.len() > CELL_COLORS {
                busy += 1;
            }
        }
    }
    busy as f64 / (GRID * GRID) as f64
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
