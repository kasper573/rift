use std::collections::HashSet;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thirtyfour::LoggingPrefsLogLevel;
use thirtyfour::prelude::*;

const ARTIFACTS: &str = env!("CARGO_TARGET_TMPDIR");
const CHROMEDRIVER: &str = "http://127.0.0.1:9515";
const DEFAULT_URL: &str = "https://rift.localhost";

// The reference snapshots were captured at this canvas size, and the portal template matches at that
// exact pixel scale (the game upscales its fixed-height view to the canvas). The harness resizes the
// browser so the canvas matches, independent of the surrounding chrome.
const CANVAS_W: i64 = 1156;
const CANVAS_H: i64 = 862;

const SCENE_CELLS: f64 = 0.3;
const GRID: u16 = 8;
const CELL_COLORS: usize = 4;
const WALKED: f64 = 0.2;

// How long a click holds the mouse button down. The client only acts on a click it sees held during
// a frame (`click_to_act` samples the button as pressed), and an instant press+release can fall
// between frames on the headless software renderer; this outlasts a frame, short of move-repeat.
const CLICK_HOLD: Duration = Duration::from_millis(250);

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

#[tokio::test]
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
async fn a_player_registers_and_visibly_walks() {
    let session = register_and_spawn().await;
    let before = session.scene.clone();
    save(&before, "before");

    let (width, height) = (before.width as i32, before.height as i32);
    let mut moved = 0.0;
    for (dx, dy) in [(200, 0), (0, 150), (-200, 0), (0, -150)] {
        session.click(width / 2 + dx, height / 2 + dy).await;
        sleep(Duration::from_secs(2)).await;
        let after = session.capture().await;
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

#[tokio::test]
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
async fn spawns_into_the_expected_island_scene() {
    let session = register_and_spawn().await;
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

#[tokio::test]
#[ignore = "e2e: drives the live stack; run with `just e2e`"]
async fn walking_through_a_portal_crosses_to_the_forest() {
    let session = register_and_spawn().await;
    let island = load_reference(ISLAND_SNAPSHOT);
    let forest = load_reference(FOREST_SNAPSHOT);
    save(&session.scene, "portal-before");

    let on_island = resemblance(&session.scene, &island);
    assert!(
        on_island > resemblance(&session.scene, &forest),
        "the player should start in the island (island resemblance {on_island:.3}); \
         see {ARTIFACTS}/portal-before.png",
    );

    let after = cross_island_portal(&session, &island, &forest).await;
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
/// browser open for the duration of the test (its canvas is the game).
struct Session {
    driver: WebDriver,
    canvas: WebElement,
    scene: Image,
}

async fn register_and_spawn() -> Session {
    let driver = new_driver().await;
    let base = base_url();
    driver
        .goto(format!("{base}/play"))
        .await
        .expect("navigate to /play");
    register(&driver).await;

    let canvas = wait_for_canvas(&driver).await;
    fit_canvas(&driver, &canvas).await;
    let mut session = Session {
        scene: capture(&canvas).await,
        driver,
        canvas,
    };
    session.scene = wait_for_scene(&session).await;
    session
}

async fn new_driver() -> WebDriver {
    let mut caps = DesiredCapabilities::chrome();
    for arg in [
        "--headless=new",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        // Caddy serves a local CA the browser doesn't trust; the test isn't checking TLS.
        "--ignore-certificate-errors",
        // Roughly sized so the canvas lands near the snapshot resolution; `fit_canvas` fine-tunes it.
        "--window-size=1156,1062",
        "--force-device-scale-factor=1",
        // Software WebGL2 so the game renders without a GPU on CI.
        "--use-gl=angle",
        "--use-angle=swiftshader",
        "--enable-unsafe-swiftshader",
    ] {
        caps.add_arg(arg).expect("chrome arg");
    }
    caps.set_logging_prefs("browser", LoggingPrefsLogLevel::All)
        .expect("enable browser logging");
    WebDriver::new(CHROMEDRIVER, caps)
        .await
        .expect("connect to chromedriver (is it running? see `just e2e`)")
}

/// Prints the browser console — the wasm client logs (and panics) here, the only window into why a
/// scene never rendered.
async fn dump_console(driver: &WebDriver) {
    if let Ok(entries) = driver.browser_log().await {
        for entry in entries {
            println!("[browser:{}] {}", entry.level, entry.message);
        }
    }
}

fn base_url() -> String {
    std::env::var("RIFT_E2E_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

/// Signs in by registering a fresh throwaway account: from signed-out `/play`, follow the sign-in
/// link to Keycloak, jump straight to its registration form, fill it, and submit. Keycloak then
/// redirects back through the website's callback to `/play`, now signed in.
async fn register(driver: &WebDriver) {
    driver
        .find(By::Css(".centered button"))
        .await
        .expect("the signed-out /play sign-in button")
        .click()
        .await
        .expect("click sign-in");

    let authorize = wait_for_url(driver, "/protocol/openid-connect/auth").await;
    let registration = authorize.replace(
        "/protocol/openid-connect/auth?",
        "/protocol/openid-connect/registrations?",
    );
    driver
        .goto(registration)
        .await
        .expect("open the registration form");

    let stamp = unix_now().as_secs();
    let user = format!("tester{stamp}");
    let password = format!("e2e-{stamp}-pw");
    fill(driver, "username", &user).await;
    fill(driver, "email", &format!("{user}@example.com")).await;
    fill(driver, "password", &password).await;
    fill(driver, "password-confirm", &password).await;
    fill_if_present(driver, "firstName", &user).await;
    fill_if_present(driver, "lastName", &user).await;

    driver
        .find(By::Css("input[type=submit]"))
        .await
        .expect("the registration submit button")
        .click()
        .await
        .expect("submit registration");
    println!("registered {user}; waiting for the redirect to /play");
    wait_for_url(driver, "/play").await;
}

async fn fill(driver: &WebDriver, id: &str, value: &str) {
    driver
        .find(By::Id(id))
        .await
        .unwrap_or_else(|_| panic!("registration field #{id}"))
        .send_keys(value)
        .await
        .unwrap_or_else(|_| panic!("fill #{id}"));
}

async fn fill_if_present(driver: &WebDriver, id: &str, value: &str) {
    if let Ok(field) = driver.find(By::Id(id)).await {
        let _ = field.send_keys(value).await;
    }
}

async fn wait_for_url(driver: &WebDriver, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(url) = driver.current_url().await
            && url.as_str().contains(needle)
        {
            return url.into();
        }
        assert!(
            Instant::now() < deadline,
            "the page never navigated to a url containing `{needle}`",
        );
        sleep(Duration::from_millis(250)).await;
    }
}

/// Resizes the browser so the game canvas matches the reference snapshots' resolution, absorbing
/// whatever chrome (the nav bar) surrounds it into the window size.
async fn fit_canvas(driver: &WebDriver, canvas: &WebElement) {
    let window = driver.get_window_rect().await.expect("window rect");
    let rect = canvas.rect().await.expect("canvas rect");
    let width = (window.width + (CANVAS_W - rect.width as i64)).max(1) as u32;
    let height = (window.height + (CANVAS_H - rect.height as i64)).max(1) as u32;
    driver
        .set_window_rect(window.x, window.y, width, height)
        .await
        .expect("resize window");
    // Let the layout settle and the client re-render at the new canvas size.
    sleep(Duration::from_millis(500)).await;
}

async fn wait_for_canvas(driver: &WebDriver) -> WebElement {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(canvas) = driver.find(By::Id("glcanvas")).await {
            return canvas;
        }
        assert!(
            Instant::now() < deadline,
            "the game canvas never appeared on /play",
        );
        sleep(Duration::from_millis(500)).await;
    }
}

impl Session {
    /// Captures the game canvas.
    async fn capture(&self) -> Image {
        capture(&self.canvas).await
    }

    /// Clicks the canvas at a point in capture-pixel coordinates (origin top-left), holding the
    /// button down long enough for the client to sample it. WebDriver offsets are from the element
    /// center, so we recentre and scale capture pixels to the element's CSS pixels.
    async fn click(&self, x: i32, y: i32) {
        let rect = self.canvas.rect().await.expect("canvas rect");
        // The displayed canvas can differ in size from its backing buffer (what we screenshot and
        // locate in), and that ratio only settles after the initial layout, so derive it fresh each
        // click rather than caching a value that may be stale by the time we act.
        let scale_x = rect.width / self.scene.width as f64;
        let scale_y = rect.height / self.scene.height as f64;
        let offset_x = (x as f64 * scale_x - rect.width / 2.0) as i64;
        let offset_y = (y as f64 * scale_y - rect.height / 2.0) as i64;
        self.driver
            .action_chain()
            .move_to_element_with_offset(&self.canvas, offset_x, offset_y)
            .click_and_hold()
            .perform()
            .await
            .expect("press on canvas");
        sleep(CLICK_HOLD).await;
        self.driver
            .action_chain()
            .release()
            .perform()
            .await
            .expect("release on canvas");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Always tear the browser down, even when a test panics mid-assertion: a leaked WebDriver
        // session keeps a headless Chrome (and the game, audio and all) alive, and a killed
        // chromedriver orphans rather than reaps it. `quit` is async and we may be inside the test's
        // runtime, so drive it to completion on a throwaway runtime on its own thread.
        let driver = self.driver.clone();
        let _ = std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Runtime::new() {
                let _ = runtime.block_on(driver.quit());
            }
        })
        .join();
    }
}

async fn capture(canvas: &WebElement) -> Image {
    let png = canvas.screenshot_as_png().await.expect("canvas screenshot");
    decode(std::io::Cursor::new(png))
}

/// Walks the player through the island's warp and returns the scene once it resembles the forest.
/// Finds the portal marker in the live frame and clicks its centre: that lands inside the 1-tile warp
/// rect, so the server's `MoveToPortal` paths the player onto the portal and crosses. The marker is
/// re-found each pass — the camera shifts as the player walks toward it.
async fn cross_island_portal(session: &Session, island: &Image, forest: &Image) -> Image {
    let template = load_template(PORTAL_TEMPLATE);
    let crossed = |cap: &Image| {
        let on_forest = resemblance(cap, forest);
        on_forest >= RESEMBLANCE && on_forest > resemblance(cap, island)
    };
    // Acquire the portal before trying to cross: if the spawn scene never shows it, the run is broken
    // in a way no amount of clicking fixes, so fail fast and loudly instead of timing out.
    let acquire = Instant::now() + Duration::from_secs(10);
    let mut target = loop {
        let cap = session.capture().await;
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
        sleep(Duration::from_millis(500)).await;
    };

    // The warp is one tile; a software renderer can place the marker a touch off, landing the click on
    // a neighbouring tile (a plain move, no cross). Sweep a half-tile cross around the located centre so
    // one click reliably lands inside the rect.
    let step = (session.scene.height as i32 / 36).max(1);
    let sweep = [(0, 0), (step, 0), (-step, 0), (0, step), (0, -step)];
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        for (dx, dy) in sweep {
            session.click(target.0 + dx, target.1 + dy).await;
            for _ in 0..3 {
                sleep(Duration::from_millis(500)).await;
                let cap = session.capture().await;
                if crossed(&cap) {
                    return cap;
                }
            }
        }
        let cap = session.capture().await;
        if Instant::now() >= deadline {
            save(&cap, "portal-timeout");
            return cap;
        }
        if let Some(point) = locate(&cap, &template) {
            target = point;
        }
    }
}

async fn wait_for_scene(session: &Session) -> Image {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let image = session.capture().await;
        let scenery = scene_fraction(&center(&image));
        if scenery >= SCENE_CELLS {
            println!(
                "the world is on screen ({:.0}% of mid-view cells show scenery)",
                scenery * 100.0
            );
            sleep(Duration::from_millis(300)).await;
            return session.capture().await;
        }
        if Instant::now() >= deadline {
            save(&image, "timeout");
            dump_console(&session.driver).await;
            panic!(
                "the world never appeared: {:.0}% of mid-view cells show scenery, a scene fills at \
                 least {:.0}% (see {ARTIFACTS}/timeout.png)",
                scenery * 100.0,
                SCENE_CELLS * 100.0,
            );
        }
        sleep(Duration::from_millis(500)).await;
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

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after the unix epoch")
}

async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[derive(Clone)]
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
    let file = std::fs::File::create(artifacts().join(format!("{name}.png"))).expect("create png");
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
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    decode(BufReader::new(file))
}

/// Decodes a PNG (a reference snapshot or a live canvas screenshot) into an RGB [`Image`].
fn decode<R: std::io::BufRead + std::io::Seek>(source: R) -> Image {
    let mut reader = png::Decoder::new(source).read_info().expect("png info");
    let mut buf = vec![0; reader.output_buffer_size().expect("png buffer size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let rgb = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()]
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()].to_vec(),
        other => panic!("unsupported png color type {other:?}"),
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
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
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
