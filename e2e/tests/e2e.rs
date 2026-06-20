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

// The portal marker sprite, cropped from a scene. The test finds it in the live frame and clicks its
// center rather than aiming at hardcoded tile coordinates, so it tracks the portal wherever it lands.
const PORTAL_TEMPLATE: &str = "portal.png";
// Mean per-channel-sum difference over the template's opaque pixels. A match scores near 0; an absent
// portal scores in the hundreds (measured: ~270 on a portal-less frame), so this cleanly splits them.
const PORTAL_MATCH: f64 = 120.0;

#[test]
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
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
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
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
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
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

/// Walks the player through the island's warp and returns the scene once it resembles the forest.
/// Finds the portal marker in the live frame and clicks its center: that lands inside the 1-tile warp
/// rect, so the server's `MoveToPortal` paths the player onto the portal and crosses. The marker is
/// re-found each pass — the camera shifts as the player walks, and a click swallowed by a roaming NPC
/// just retries against the unchanged scene.
fn cross_island_portal(enigo: &mut Enigo, game: &Win, island: &Image, forest: &Image) -> Image {
    let template = load_template(PORTAL_TEMPLATE);
    let crossed = |cap: &Image| {
        let on_forest = resemblance(cap, forest);
        on_forest >= RESEMBLANCE && on_forest > resemblance(cap, island)
    };

    // Acquire the portal before trying to cross: if the spawn scene never shows it, the run is broken
    // in a way no amount of clicking fixes, so fail fast and loudly instead of timing out.
    let acquire = Instant::now() + Duration::from_secs(10);
    let mut target = loop {
        let cap = game.capture();
        if let Some(point) = locate(&cap, &template) {
            break point;
        }
        if Instant::now() >= acquire {
            save(&cap, "portal-not-found");
            panic!(
                "the portal marker ({PORTAL_TEMPLATE}) never appeared in the spawn scene; the player \
                 may not have spawned on the island (see {ARTIFACTS}/portal-not-found.png)",
            );
        }
        sleep(Duration::from_millis(500));
    };

    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        game.click(enigo, target.0, target.1);
        for _ in 0..8 {
            sleep(Duration::from_millis(500));
            let cap = game.capture();
            if crossed(&cap) {
                return cap;
            }
        }
        let cap = game.capture();
        if Instant::now() >= deadline {
            save(&cap, "portal-timeout");
            return cap;
        }
        if let Some(point) = locate(&cap, &template) {
            target = point;
        }
    }
}

/// The center, in capture pixels, of the best match for `template` in `cap` — or `None` when nothing
/// resembles it closely enough (see `PORTAL_MATCH`). Coarse-to-fine masked matching: a stride-4 sweep
/// locates the region, then a 1px refinement around it; only the template's opaque pixels count.
fn locate(cap: &Image, template: &Template) -> Option<(i32, i32)> {
    const COARSE: usize = 4;
    let (tw, th) = (template.width as usize, template.height as usize);
    let (cw, ch) = (cap.width as usize, cap.height as usize);
    if cw <= tw || ch <= th {
        return None;
    }
    let mut best = (u64::MAX, 0usize, 0usize);

    let mut y = 0;
    while y <= ch - th {
        let mut x = 0;
        while x <= cw - tw {
            let score = template.score(cap, x, y, best.0);
            if score < best.0 {
                best = (score, x, y);
            }
            x += COARSE;
        }
        y += COARSE;
    }

    let (x0, x1) = (
        best.1.saturating_sub(COARSE),
        (best.1 + COARSE).min(cw - tw),
    );
    let (y0, y1) = (
        best.2.saturating_sub(COARSE),
        (best.2 + COARSE).min(ch - th),
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            let score = template.score(cap, x, y, best.0);
            if score < best.0 {
                best = (score, x, y);
            }
        }
    }

    let mean = best.0 as f64 / template.samples.len() as f64;
    (mean <= PORTAL_MATCH).then(|| ((best.1 + tw / 2) as i32, (best.2 + th / 2) as i32))
}

/// A sprite reduced to its opaque pixels, for matching against a capture by sum-of-absolute-difference.
struct Template {
    width: u16,
    height: u16,
    samples: Vec<([u16; 2], [u8; 3])>,
}

impl Template {
    /// Total per-channel absolute difference over the opaque samples placed at `(ox, oy)`, abandoned
    /// early once it passes `cutoff` (it can only grow), which is what keeps the full sweep cheap.
    fn score(&self, cap: &Image, ox: usize, oy: usize, cutoff: u64) -> u64 {
        let mut sum = 0u64;
        for ([dx, dy], rgb) in &self.samples {
            let i = 3 * ((oy + *dy as usize) * cap.width as usize + ox + *dx as usize);
            let d = |a: u8, b: u8| (a as i32 - b as i32).unsigned_abs() as u64;
            sum += d(cap.rgb[i], rgb[0]) + d(cap.rgb[i + 1], rgb[1]) + d(cap.rgb[i + 2], rgb[2]);
            if sum >= cutoff {
                return sum;
            }
        }
        sum
    }
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

// Every other opaque pixel is plenty to identify the marker and halves the matching work.
const TEMPLATE_STRIDE: usize = 2;

fn load_template(name: &str) -> Template {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join(name);
    let file = File::open(&path).unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .expect("png info");
    let mut buf = vec![0; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    assert_eq!(
        info.color_type,
        png::ColorType::Rgba,
        "template {name} needs an alpha channel to mask its background"
    );
    let (width, height) = (info.width as u16, info.height as u16);
    let samples = buf[..info.buffer_size()]
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[3] > 128)
        .step_by(TEMPLATE_STRIDE)
        .map(|(i, p)| {
            let (x, y) = ((i % width as usize) as u16, (i / width as usize) as u16);
            ([x, y], [p[0], p[1], p[2]])
        })
        .collect();
    Template {
        width,
        height,
        samples,
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
