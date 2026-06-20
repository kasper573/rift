use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arboard::Clipboard;
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use xcap::Window;

const ARTIFACTS: &str = env!("CARGO_TARGET_TMPDIR");
const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets");
const WINDOW_TITLE: &str = "rift mmo";
const BROWSER_TITLE: &str = "sign in to rift";
const ISSUER: &str = "https://auth.rift.localhost/realms/rift";
const AUDIENCE: &str = "rift";

const SCENE_CELLS: f64 = 0.3;
const GRID: u16 = 8;
const CELL_COLORS: usize = 4;
const WALKED: f64 = 0.2;

// A color-histogram intersection at or above this means the scene "shows basically the same place"
// as a reference snapshot — tolerant of the player's exact position and the NPCs milling about.
const RESEMBLANCE: f64 = 0.5;
const ISLAND_SNAPSHOT: &str = "spawn-on-island.png";
const FOREST_SNAPSHOT: &str = "spawn-on-forest.png";

// Geometry for aiming a portal click, in tiles. Tiles per screen height must match
// `client::render::VIEW_TILES_TALL`; the camera centers on the player. The island's spawn and the
// nearest warp (which leads to the forest) come from `assets/maps/island.tmx`.
const VIEW_TILES_TALL: f64 = 18.0;
// Tile centers: the spawn (`start` object's tile) and the nearest warp's 1-tile rect — so the warp
// sits straight up, 3 tiles north of where the player spawns.
const ISLAND_SPAWN: (f64, f64) = (39.5, 29.5);
const ISLAND_PORTAL: (f64, f64) = (39.5, 26.5);

#[test]
#[ignore = "e2e: needs a display, a browser and the stack; CI-only, run with `just e2e`"]
fn a_player_registers_and_visibly_walks() {
    let mut session = register_and_spawn();
    let before = session.scene;
    save(&before, "before");

    let (width, height) = (before.width as i32, before.height as i32);
    let mut moved = 0.0;
    for (dx, dy) in [(200, 0), (0, 150), (-200, 0), (0, -150)] {
        tap_space(&mut session.enigo);
        session
            .game
            .click(&mut session.enigo, width / 2 + dx, height / 2 + dy);
        sleep(Duration::from_secs(2));
        let after = session.game.capture();
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

#[test]
#[ignore = "e2e: needs a display, a browser and the stack; CI-only, run with `just e2e`"]
fn spawns_into_the_expected_island_scene() {
    let session = register_and_spawn();
    save(&session.scene, "island-spawn");
    let island = load_reference(ISLAND_SNAPSHOT);
    let forest = load_reference(FOREST_SNAPSHOT);
    let on_island = resemblance(&session.scene, &island);
    let on_forest = resemblance(&session.scene, &forest);
    println!("spawn resemblance: island {on_island:.3}, forest {on_forest:.3}");
    assert!(
        on_island >= RESEMBLANCE && on_island > on_forest,
        "the spawn scene should resemble the island snapshot (island {on_island:.3}, forest \
         {on_forest:.3}; need >= {RESEMBLANCE:.2} and island > forest; see {ARTIFACTS}/island-spawn.png)",
    );
}

#[test]
#[ignore = "e2e: needs a display, a browser and the stack; CI-only, run with `just e2e`"]
fn walking_through_a_portal_crosses_to_the_forest() {
    let mut session = register_and_spawn();
    let island = load_reference(ISLAND_SNAPSHOT);
    let forest = load_reference(FOREST_SNAPSHOT);
    save(&session.scene, "portal-before");

    let on_island = resemblance(&session.scene, &island);
    assert!(
        on_island > resemblance(&session.scene, &forest),
        "the player should start in the island (island resemblance {on_island:.3}); \
         see {ARTIFACTS}/portal-before.png",
    );

    let after = cross_island_portal(&mut session.enigo, &session.game, &island, &forest);
    save(&after, "portal-after");
    let on_forest = resemblance(&after, &forest);
    let on_island = resemblance(&after, &island);
    println!("after crossing: forest {on_forest:.3}, island {on_island:.3}");
    assert!(
        on_forest >= RESEMBLANCE && on_forest > on_island,
        "walking through the portal should show the forest (forest {on_forest:.3}, island \
         {on_island:.3}; need >= {RESEMBLANCE:.2} and forest > island; see {ARTIFACTS}/portal-after.png)",
    );
}

/// A registered, signed-in session sitting in the game with the spawn scene captured. Holds the
/// server and client processes alive for the duration of the test.
struct Session {
    _server: Option<GameServer>,
    _client: Proc,
    enigo: Enigo,
    game: Win,
    scene: Image,
}

fn register_and_spawn() -> Session {
    let server = (!prod()).then(GameServer::start);
    let client = spawn_client(server.as_ref().map(|server| server.url.as_str()));
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
    let scene = wait_for_scene(&game, Duration::from_secs(120));
    Session {
        _server: server,
        _client: client,
        enigo,
        game,
        scene,
    }
}

/// Walks the player from the island spawn through the nearest warp and returns the scene once it
/// resembles the forest. A `MoveToPortal` click (one landing inside the 1-tile warp rect) makes the
/// server path the player onto the portal and cross; the first aim is the warp seen from spawn, then,
/// since a miss leaves the player walked-toward-it, a small grid around the now-near warp.
fn cross_island_portal(enigo: &mut Enigo, game: &Win, island: &Image, forest: &Image) -> Image {
    let crossed = |cap: &Image| {
        let on_forest = resemblance(cap, forest);
        on_forest >= RESEMBLANCE && on_forest > resemblance(cap, island)
    };
    let ppt = game.capture().height as f64 / VIEW_TILES_TALL;
    let aim = |enigo: &mut Enigo, tx: f64, ty: f64| {
        let cap = game.capture();
        let x = (cap.width as f64 / 2.0 + tx * ppt).round() as i32;
        let y = (cap.height as f64 / 2.0 + ty * ppt).round() as i32;
        game.click(enigo, x, y);
    };
    let poll = |secs: u64| -> Option<Image> {
        let until = Instant::now() + Duration::from_secs(secs);
        loop {
            sleep(Duration::from_millis(500));
            let cap = game.capture();
            if crossed(&cap) {
                return Some(cap);
            }
            if Instant::now() >= until {
                return None;
            }
        }
    };

    // First aim: the warp as seen from the spawn, where the player still stands.
    let (wx, wy) = (
        ISLAND_PORTAL.0 - ISLAND_SPAWN.0,
        ISLAND_PORTAL.1 - ISLAND_SPAWN.1,
    );
    aim(enigo, wx, wy);
    if let Some(cap) = poll(8) {
        return cap;
    }

    // A miss walks the player toward the warp; nudge it onto the 1-tile rect with fine, mostly
    // northward steps (smaller than a tile so a click can't skip over the rect), polling each.
    let sweep = [
        (0.0, -0.35),
        (0.0, -0.7),
        (0.0, 0.0),
        (-0.35, -0.5),
        (0.35, -0.5),
        (0.0, -1.05),
        (-0.35, -1.0),
        (0.35, -1.0),
        (0.0, 0.4),
        (-0.4, 0.0),
        (0.4, 0.0),
        (0.0, -1.4),
    ];
    for (ox, oy) in sweep {
        aim(enigo, ox, oy);
        if let Some(cap) = poll(4) {
            return cap;
        }
    }
    let cap = game.capture();
    save(&cap, "portal-timeout");
    cap
}

fn register_in_browser(enigo: &mut Enigo) {
    let register_url = wait_for_authorize_url(Duration::from_secs(60)).replace(
        "/protocol/openid-connect/auth?",
        "/protocol/openid-connect/registrations?",
    );
    let browser = wait_for_browser(Duration::from_secs(60));
    // Paste the URL into the address bar rather than type it: a 40-character PKCE code_challenge sent key-by-key loses characters on a loaded X server. The clipboard transfers the whole URL atomically.
    let mut clipboard = Clipboard::new().expect("open clipboard");
    clipboard.set_text(&register_url).expect("set clipboard");
    browser.click(enigo, 100, 10);
    chord(enigo, Key::Control, Key::Unicode('l'));
    chord(enigo, Key::Control, Key::Unicode('v'));
    tap(enigo, Key::Delete);
    tap(enigo, Key::Return);
    sleep(Duration::from_secs(8));

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
        // Paste each value: typed key-by-key, a field can lose a character under load.
        clipboard.set_text(value).expect("set clipboard");
        chord(enigo, Key::Control, Key::Unicode('v'));
        sleep(Duration::from_millis(100));
    }
    tap(enigo, Key::Tab);
    tap(enigo, Key::Return);
    println!("registered {user}; waiting for the redirect to the client");
    sleep(Duration::from_secs(8));
    save(&browser.capture(), "register-result");

    chord(enigo, Key::Control, Key::Unicode('w'));
    sleep(Duration::from_secs(1));
}

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
    // Pre-mark chrome's first run to keep its welcome dialog from covering the page.
    let config = artifacts().join("config");
    std::fs::create_dir_all(config.join("google-chrome")).expect("chrome config dir");
    let _ = File::create(config.join("google-chrome").join("First Run"));

    let mut command = Command::new(client_bin());
    command
        // Never let a stray Wayland socket pull the window elsewhere.
        .env_remove("WAYLAND_DISPLAY")
        .env("XDG_CONFIG_HOME", config);
    if let Some(url) = game_server_url {
        command
            .env("RIFT_CLIENT_ISSUER", ISSUER)
            .env("RIFT_CLIENT_GAME_SERVER_URL", url)
            .env("RIFT_CLIENT_OIDC_CLIENT_ID", AUDIENCE)
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
            .env("RIFT_AUTH_AUDIENCE", AUDIENCE)
            .env(
                "RIFT_AUTH_JWKS_URI",
                format!("{ISSUER}/protocol/openid-connect/certs"),
            )
            .env("RIFT_GAME_SERVER_PUBLIC_HOST", "127.0.0.1")
            .env("RIFT_GAME_SERVER_PYROSCOPE_ENABLED", "false")
            .env("RIFT_GAME_SERVER_PYROSCOPE_SAMPLE_HZ", "99")
            // Survive the scenario: with a normal 30 HP the island's NPCs kill the player before it
            // can walk a portal, and the death overlay tints the whole scene.
            .env("RIFT_GAME_SERVER_PLAYER_HEALTH", "1000000");
        let proc = Proc::start(command, "server");

        // Verify tokens before returning; this also waits out the stack's keycloak.
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

struct Image {
    width: u16,
    height: u16,
    rgb: Vec<u8>,
}

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

fn load_reference(name: &str) -> Image {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join(name);
    let file = File::open(&path).unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .expect("png info");
    let mut buf = vec![0; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let rgb = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()]
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()].to_vec(),
        other => panic!("snapshot {name} has unsupported color type {other:?}"),
    };
    Image {
        width: info.width as u16,
        height: info.height as u16,
        rgb,
    }
}

const HIST_BINS: usize = 8;

fn color_histogram(image: &Image) -> [f64; HIST_BINS * HIST_BINS * HIST_BINS] {
    let mut hist = [0f64; HIST_BINS * HIST_BINS * HIST_BINS];
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return hist;
    }
    let samples = 64usize;
    for ty in 0..samples {
        for tx in 0..samples {
            let sx = (tx * w / samples).min(w - 1);
            let sy = (ty * h / samples).min(h - 1);
            let i = 3 * (sy * w + sx);
            let bin = |c: u8| (c as usize * HIST_BINS / 256).min(HIST_BINS - 1);
            let (r, g, b) = (
                bin(image.rgb[i]),
                bin(image.rgb[i + 1]),
                bin(image.rgb[i + 2]),
            );
            hist[(r * HIST_BINS + g) * HIST_BINS + b] += 1.0;
        }
    }
    let total = (samples * samples) as f64;
    for value in hist.iter_mut() {
        *value /= total;
    }
    hist
}

/// Color-histogram intersection in [0, 1]: 1 is an identical color distribution, 0 no overlap.
/// Coarsely samples to 64x64 with 8 bins per channel so it answers "is this the same place",
/// tolerant of the player's exact position and roaming NPCs rather than exact pixels (ported from
/// the pre-Rust suite's `image-resemblance`).
fn resemblance(a: &Image, b: &Image) -> f64 {
    let (ha, hb) = (color_histogram(a), color_histogram(b));
    ha.iter().zip(hb.iter()).map(|(x, y)| x.min(*y)).sum()
}

fn artifacts() -> PathBuf {
    let dir = PathBuf::from(ARTIFACTS);
    std::fs::create_dir_all(&dir).expect("artifacts dir");
    dir
}
