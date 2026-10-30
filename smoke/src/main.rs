//! The website and the browser client, smoke-tested together from a binaries folder: a player
//! seeded in a stand-in identity provider signs in on the site of a game server and plays, and
//! the canvas must come to look like the trusted snapshot of the game's first view.

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::{Browser, BrowserConfig, Page};
use engine::core::content::FilesystemFolder;
use engine::core::time::UtcHour;
use engine::systems::launch::{
    self, AuthMode, ReadyLine, SERVER_ENV_PREFIX, ServerConfig, ServerDaylight, ServerReady,
};
use engine::systems::player::SpawnPolicy;
use engine::systems::{GameSettings, load_content};
use futures::StreamExt;
use image::imageops::FilterType;
use image::{RgbImage, RgbaImage};
use oauth2_test_server::{IssuerConfig, OAuthTestServer};
use openidconnect::{
    AuthUrl, ClientId, EndSessionUrl, IssuerUrl, JsonWebKeySetUrl, RedirectUrl, TokenUrl,
};
use serde::Serialize;
use web_server::{DESKTOP_SIGN_IN_ENV_PREFIX, SignInSettings, WebServerConfig};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let binaries = args
        .next()
        .map(PathBuf::from)
        .expect("usage: smoke <binaries folder> [--snapshot]");
    let taking_snapshot = args.next().as_deref() == Some("--snapshot");
    let binaries = std::path::absolute(&binaries).expect("the binaries folder");
    let scratch = tempfile::tempdir().expect("a scratch folder");

    let provider = OAuthTestServer::start_with_config(IssuerConfig {
        host: LOCALHOST.to_string(),
        default_user_id: PLAYER.to_owned(),
        ..IssuerConfig::default()
    })
    .await;
    let site_port = free_port();
    let site = format!("http://{LOCALHOST}:{site_port}");
    let client = provider
        .register_client(
            serde_json::to_value(Registration {
                scope: "openid profile",
                redirect_uris: vec![format!("{site}/auth-callback")],
                token_endpoint_auth_method: "none",
            })
            .expect("a registration"),
        )
        .await
        .client_id;
    let sign_in = oidc::TokenCheckSettings {
        issuer: provider.issuer().to_owned(),
        client_id: client.clone(),
        jwks_uri: provider.endpoints.jwks.clone(),
    };

    let (_game_server, game_server) = game_server(&binaries.join("game-server"), &sign_in);
    let wasm = scratch.path().join("wasm");
    unpack(&binaries.join("wasm-client.tar.gz"), &wasm);
    let _web_server = web_server(
        &binaries.join("web-server"),
        &site,
        site_port,
        game_server,
        &wasm,
        &provider,
        &client,
    );

    let player = Player::sign_in(&site).await;
    player.arrive(&spawn_area()).await;
    if taking_snapshot {
        let settled = player.settled().await;
        settled.save(snapshot_path()).expect("the snapshot written");
        println!(
            "the settled canvas is the new snapshot, {}: look at it before trusting it",
            snapshot_path().display()
        );
    } else {
        let snapshot = image::open(snapshot_path())
            .expect("the trusted snapshot: take it with --snapshot")
            .to_rgba8();
        let (resemblance, seen) = player.until_alike(&snapshot).await;
        if resemblance < LOOKS_ALIKE {
            let kept = std::path::absolute("target/smoke/seen.png").expect("a path");
            std::fs::create_dir_all(kept.parent().expect("a folder")).expect("target/smoke");
            seen.save(&kept).expect("the canvas as seen");
            eprintln!(
                "the canvas never came to look like the snapshot (resemblance {resemblance:.2}, \
                 wanted {LOOKS_ALIKE}): the closest it came is in {}, the snapshot in {}",
                kept.display(),
                snapshot_path().display()
            );
            std::process::exit(1);
        }
        println!(
            "the seeded player signed in and the browser client drew the game (resemblance {resemblance:.2})"
        );
    }
    player.leave().await;
}

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const PLAYER: &str = "smoke";
const SEED: u64 = 7;
/// "The same picture", tolerant of those around the player moving and of how a rasterizer rounds;
/// a blank, dark or world-less canvas falls well short of it.
const LOOKS_ALIKE: f32 = 0.95;
/// How alike two looks a second apart must be for the canvas to count as still.
const STILL: f32 = 0.97;
/// How long the canvas must stay still to count as settled.
const SETTLED_FOR: u32 = 10;
/// How much of a picture one colour may fill before it counts as blank.
const BLANK: f32 = 0.9;
/// What a software renderer needs to load the game and draw it.
const DRAWN_WITHIN: Duration = Duration::from_secs(300);
const READY_WITHIN: Duration = Duration::from_secs(120);
const VIEWPORT: (u32, u32) = (800, 600);

/// A player on the site's play page in Chrome, signed in as the seeded player.
struct Player {
    browser: Browser,
    page: Page,
    events: tokio::task::JoinHandle<()>,
}

impl Player {
    async fn sign_in(site: &str) -> Player {
        let (browser, mut handler) = Browser::launch(
            BrowserConfig::builder()
                .new_headless_mode()
                .window_size(VIEWPORT.0, VIEWPORT.1)
                .viewport(Some(Viewport {
                    width: VIEWPORT.0,
                    height: VIEWPORT.1,
                    ..Viewport::default()
                }))
                // The same software renderer everywhere, so every machine draws the snapshot alike.
                .args([
                    "--use-angle=swiftshader",
                    "--enable-unsafe-swiftshader",
                    "--mute-audio",
                ])
                .build()
                .expect("a browser configuration"),
        )
        .await
        .expect("Chrome runs: the smoke test plays the browser client in it");
        let events = tokio::spawn(async move { while handler.next().await.is_some() {} });
        let page = browser
            .new_page(format!("{site}/play"))
            .await
            .expect("the play page");
        page.find_element("button")
            .await
            .expect("the sign-in button")
            .click()
            .await
            .expect("a click");
        page.wait_for_navigation().await.expect("back on the site");
        Player {
            browser,
            page,
            events,
        }
    }

    /// Waits for the client to announce the player's arrival in `area`, which it does once the
    /// game's content is loaded and the area is drawn.
    async fn arrive(&self, area: &str) {
        let deadline = Instant::now() + DRAWN_WITHIN;
        loop {
            if let Some(crash) = crash(&self.page).await {
                panic!("the browser client crashed: {crash}");
            }
            let status = match self.page.find_element("#sync-status").await {
                Ok(element) => element.inner_text().await.ok().flatten(),
                Err(_) => None,
            };
            if status.as_deref() == Some(area) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the player never arrived in {area:?}; the client last said {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Watches the canvas until it looks like `snapshot`, or the time runs out; answers how close
    /// it came, and the look that came closest.
    async fn until_alike(&self, snapshot: &RgbaImage) -> (f32, RgbaImage) {
        let deadline = Instant::now() + DRAWN_WITHIN;
        let mut closest: Option<(f32, RgbaImage)> = None;
        while Instant::now() < deadline {
            let seen = self.look().await;
            let alike = resemblance(&seen, snapshot);
            if closest.as_ref().is_none_or(|(best, _)| alike >= *best) {
                closest = Some((alike, seen));
            }
            if alike >= LOOKS_ALIKE {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        closest.expect("a look at the canvas")
    }

    /// Watches the canvas until the picture holds still, as it does once the view's reveal has
    /// played.
    async fn settled(&self) -> RgbaImage {
        let deadline = Instant::now() + DRAWN_WITHIN;
        let mut last = self.look().await;
        let mut still = 0;
        while still < SETTLED_FOR {
            assert!(
                Instant::now() < deadline,
                "the canvas never settled on a picture"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
            let seen = self.look().await;
            let held = resemblance(&seen, &last) >= STILL && !blank(&seen);
            still = if held { still + 1 } else { 0 };
            last = seen;
        }
        last
    }

    /// The canvas as it shows now. A page that reports the client crashed fails at once.
    async fn look(&self) -> RgbaImage {
        loop {
            if let Some(crash) = crash(&self.page).await {
                panic!("the browser client crashed: {crash}");
            }
            if let Some(seen) = canvas(&self.page).await {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn leave(mut self) {
        self.browser.close().await.expect("Chrome closes");
        self.events.abort();
    }
}

async fn canvas(page: &Page) -> Option<RgbaImage> {
    let png = page
        .find_element("#glcanvas")
        .await
        .ok()?
        .screenshot(CaptureScreenshotFormat::Png)
        .await
        .ok()?;
    Some(
        image::load_from_memory(&png)
            .expect("a screenshot")
            .to_rgba8(),
    )
}

/// What the page's error log shows, which it shows only when the client panicked or failed to
/// start.
async fn crash(page: &Page) -> Option<String> {
    let log = page.find_element("#error-log").await.ok()?;
    if log.attribute("hidden").await.ok()?.is_some() {
        return None;
    }
    log.inner_text().await.ok()?
}

/// How alike two pictures are, from 0 to 1: one less their mean difference in colour, seen small
/// enough that a few pixels of difference in where something stands barely count.
fn resemblance(a: &RgbaImage, b: &RgbaImage) -> f32 {
    let (a, b) = (thumbnail(a), thumbnail(b));
    let difference: u64 = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    1.0 - difference as f32 / (a.as_raw().len() as f32 * 255.0)
}

fn thumbnail(picture: &RgbaImage) -> RgbImage {
    let picture = image::DynamicImage::ImageRgba8(picture.clone()).to_rgb8();
    image::imageops::resize(&picture, THUMBNAIL.0, THUMBNAIL.1, FilterType::Triangle)
}

const THUMBNAIL: (u32, u32) = (64, 36);

/// Whether one colour fills nearly all of `picture`.
fn blank(picture: &RgbaImage) -> bool {
    histogram(picture).into_iter().any(|share| share >= BLANK)
}

const BINS: usize = 8;
const SAMPLES: u32 = 64;

fn histogram(picture: &RgbaImage) -> Vec<f32> {
    let mut counts = vec![0.0; BINS * BINS * BINS];
    let (width, height) = picture.dimensions();
    let bin = |channel: u8| channel as usize * BINS / 256;
    for y in 0..SAMPLES {
        for x in 0..SAMPLES {
            let [r, g, b, _] = picture
                .get_pixel(x * width / SAMPLES, y * height / SAMPLES)
                .0;
            counts[(bin(r) * BINS + bin(g)) * BINS + bin(b)] += 1.0;
        }
    }
    let total = (SAMPLES * SAMPLES) as f32;
    counts.iter().map(|count| count / total).collect()
}

fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshot.png")
}

/// The area a new player arrives in.
fn spawn_area() -> String {
    let content = load_content(Arc::new(FilesystemFolder(game())), "smoke")
        .unwrap_or_else(|report| panic!("the game loads: {report}"));
    let area = content.built::<GameSettings>().initial_spawn.area;
    area.get(&content).name.clone()
}

/// The game a new project starts from: a whole game, as a player sees one.
fn game() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../editor/templates/new_game")
}

#[derive(Serialize)]
struct Registration {
    scope: &'static str,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: &'static str,
}

/// A binary of the folder running, stopped when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The game server on the game, taking the seeded player's tokens, once players can connect.
fn game_server(binary: &Path, sign_in: &oidc::TokenCheckSettings) -> (Running, ServerReady) {
    let config = ServerConfig {
        bind: LOCALHOST,
        port: 0,
        ws_port: 0,
        max_clients: 8,
        content_dir: std::path::absolute(game()).expect("the game"),
        player_spawn_policy: SpawnPolicy::Map,
        player_immortal: true,
        daylight: ServerDaylight::Still,
        rng_seed: Some(SEED),
        daily_reset_utc_hour: UtcHour::MIDNIGHT,
        auth: AuthMode::Oidc,
        test_characters: false,
        exit_on_stdin_eof: true,
    };
    let mut child = Command::new(binary)
        .envs(launch::env_of(SERVER_ENV_PREFIX, &config))
        .envs(launch::env_of(oidc::ENV_PREFIX, sign_in))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{} doesn't run: {error}", binary.display()));
    let stdout = child.stdout.take().expect("a piped stdout");
    let running = Running(child);
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
        .expect("the game server gets ready");
    (running, ready)
}

/// The game server's site at `site`, signing players in on `provider`, once it answers.
fn web_server(
    binary: &Path,
    site: &str,
    port: u16,
    game_server: ServerReady,
    wasm: &Path,
    provider: &OAuthTestServer,
    client: &str,
) -> Running {
    let game_server_url = format!("http://{LOCALHOST}:{}", game_server.http);
    let config = WebServerConfig {
        bind: LOCALHOST,
        port,
        redirect_uri: RedirectUrl::new(format!("{site}/auth-callback")).expect("a url"),
        game_server_url: game_server_url.clone(),
        game_server_ws_url: format!("ws://{LOCALHOST}:{}", game_server.ws),
        game_server_internal_url: game_server_url,
        downloads_url: format!("{site}/no-downloads"),
        release: "smoke".to_owned(),
        wasm_dir: wasm.to_path_buf(),
    };
    let sign_in = SignInSettings {
        issuer: IssuerUrl::new(provider.issuer().to_owned()).expect("a url"),
        client_id: ClientId::new(client.to_owned()),
        jwks_uri: JsonWebKeySetUrl::new(provider.endpoints.jwks.clone()).expect("a url"),
        token_uri: TokenUrl::new(provider.endpoints.token.clone()).expect("a url"),
        authorization_uri: AuthUrl::new(provider.endpoints.authorize.clone()).expect("a url"),
        end_session_uri: EndSessionUrl::new(format!("{site}/")).expect("a url"),
        account_uri: None,
    };
    let desktop_sign_in = oidc::BrowserSignInSettings {
        client_id: client.to_owned(),
        authorization_uri: provider.endpoints.authorize.clone(),
        sign_up_uri: provider.endpoints.authorize.clone(),
        token_uri: provider.endpoints.token.clone(),
    };
    let running = Running(
        Command::new(binary)
            .envs(launch::env_of(web_server::ENV_PREFIX, &config))
            .envs(launch::env_of(oidc::ENV_PREFIX, &sign_in))
            .envs(launch::env_of(DESKTOP_SIGN_IN_ENV_PREFIX, &desktop_sign_in))
            .spawn()
            .unwrap_or_else(|error| panic!("{} doesn't run: {error}", binary.display())),
    );
    let since = Instant::now();
    while ureq::get(site).call().is_err() {
        assert!(
            since.elapsed() < READY_WITHIN,
            "the web server never answered"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    running
}

/// A port nothing listens on now, for a server whose address must be known before it starts.
fn free_port() -> u16 {
    TcpListener::bind((LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port()
}

fn unpack(archive: &Path, into: &Path) {
    std::fs::create_dir_all(into).expect("a folder for the browser client");
    let unpacked = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .expect("tar runs");
    assert!(unpacked.success(), "{} unpacks", archive.display());
}
