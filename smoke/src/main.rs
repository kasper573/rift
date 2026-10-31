//! The website, the game server and the desktop client smoke-tested together from a binaries
//! folder: a player seeded in a stand-in identity provider starts the desktop client from a folder
//! holding the `rift.env` the site hands out, and the window must come to look like the trusted
//! snapshot of the game's first view.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use engine::core::content::{Content, ContentScope, FilesystemFolder, with_content};
use engine::core::time::UtcHour;
use engine::systems::launch::{
    self, AuthMode, CLIENT_ENV_PREFIX, ReadyLine, SERVER_ENV_PREFIX, ServerConfig, ServerDaylight,
    ServerReady,
};
use engine::systems::player::SpawnPolicy;
use engine::systems::probe::ProbeSnapshot;
use engine::systems::{GameSettings, load_content};
use image::imageops::FilterType;
use image::{RgbImage, RgbaImage};
use oauth2_test_server::{IssuerConfig, JwtOptionsBuilder, OAuthTestServer};
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
    let reserved = TcpListener::bind((LOCALHOST, 0)).expect("a free port");
    let site_port = reserved.local_addr().expect("a bound port").port();
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
        .await;
    let token = provider
        .generate_token(
            &client,
            JwtOptionsBuilder::default()
                .user_id(PLAYER)
                .scope("openid profile")
                .expires_in(3600)
                .build(),
        )
        .await
        .access_token;
    let sign_in = oidc::TokenCheckSettings {
        issuer: provider.issuer().to_owned(),
        client_id: client.client_id.clone(),
        jwks_uri: provider.endpoints.jwks.clone(),
    };

    let (_game_server, game_server) = game_server(&binaries.join("game-server"), &sign_in);
    // Held until now so the servers started before the site's don't take its port for their own.
    drop(reserved);
    let _web_server = web_server(
        &binaries.join("web-server"),
        &site,
        site_port,
        game_server,
        &provider,
        &client.client_id,
    );

    let folder = scratch.path().join("player");
    std::fs::create_dir_all(&folder).expect("the player's folder");
    std::fs::copy(
        binaries.join("desktop-client"),
        folder.join("desktop-client"),
    )
    .expect("the desktop client in the player's folder");
    let rift_env = ureq::get(format!("{site}/rift.env"))
        .call()
        .expect("the site hands out rift.env")
        .body_mut()
        .read_to_string()
        .expect("rift.env reads");
    std::fs::write(folder.join(launch::SETTINGS_FILE), rift_env).expect("rift.env saved");

    let display = Display::start();
    let driver = TcpListener::bind((LOCALHOST, 0)).expect("a port for the probe");
    let _client = desktop_client(
        &folder,
        &token,
        &display,
        driver.local_addr().expect("a bound port"),
    );
    let mut probe = Probe::accept(&driver);
    let content = game_content();
    probe.arrive(&content);

    let pictures = scratch.path().join("pictures");
    std::fs::create_dir_all(&pictures).expect("a folder for pictures");
    if taking_snapshot {
        let settled = probe.settled(&pictures);
        settled.save(snapshot_path()).expect("the snapshot written");
        println!(
            "the settled window is the new snapshot, {}: look at it before trusting it",
            snapshot_path().display()
        );
        return;
    }
    let snapshot = image::open(snapshot_path())
        .expect("the trusted snapshot: take it with --snapshot")
        .to_rgba8();
    let (resemblance, seen) = probe.until_alike(&snapshot, &pictures);
    if resemblance < LOOKS_ALIKE {
        let kept = std::path::absolute("target/smoke/seen.png").expect("a path");
        std::fs::create_dir_all(kept.parent().expect("a folder")).expect("target/smoke");
        seen.save(&kept).expect("the window as seen");
        eprintln!(
            "the window never came to look like the snapshot (resemblance {resemblance:.2}, \
             wanted {LOOKS_ALIKE}): the closest it came is in {}, the snapshot in {}",
            kept.display(),
            snapshot_path().display()
        );
        std::process::exit(1);
    }
    println!(
        "the seeded player joined from the site's rift.env and the desktop client drew the game \
         (resemblance {resemblance:.2})"
    );
}

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const PLAYER: &str = "smoke";
const SEED: u64 = 7;
/// "The same picture", tolerant of those around the player moving and of how a rasterizer rounds;
/// a blank, dark or world-less window falls well short of it.
const LOOKS_ALIKE: f32 = 0.95;
/// How alike two looks a second apart must be for the window to count as still.
const STILL: f32 = 0.97;
/// How long the window must stay still to count as settled.
const SETTLED_FOR: u32 = 10;
/// How much of a picture one colour may fill before it counts as blank.
const BLANK: f32 = 0.9;
/// What a software renderer needs to load the game and draw it.
const DRAWN_WITHIN: Duration = Duration::from_secs(300);
const READY_WITHIN: Duration = Duration::from_secs(120);
const SCREEN: (u32, u32) = (1280, 720);
/// The software rasterizer every machine has, so every machine draws the snapshot alike.
const RASTERIZER: &str = "llvmpipe";

/// The desktop client's probe, as its driver.
struct Probe {
    questions: TcpStream,
    answers: BufReader<TcpStream>,
}

impl Probe {
    fn accept(driver: &TcpListener) -> Probe {
        let (stream, _) = driver
            .accept()
            .expect("the desktop client's probe connects");
        let answers = BufReader::new(stream.try_clone().expect("a second handle on the probe"));
        Probe {
            questions: stream,
            answers,
        }
    }

    fn ask(&mut self, question: &serde_json::Value) -> String {
        writeln!(self.questions, "{question}").expect("the probe takes a question");
        let mut answer = String::new();
        self.answers
            .read_line(&mut answer)
            .expect("the probe answers");
        assert!(!answer.is_empty(), "the desktop client hung up");
        answer
    }

    fn snapshot(&mut self, content: &Content) -> ProbeSnapshot {
        let answer = self.ask(&serde_json::json!({ "ask": "snapshot" }));
        let (snapshot, _) = with_content(ContentScope::of(content), || {
            serde_json::from_str(&answer).expect("a snapshot")
        });
        snapshot
    }

    /// Waits for the player to stand in the game's spawn area, which they do once its content is
    /// loaded and the area is drawn.
    fn arrive(&mut self, content: &Content) {
        let area = content.built::<GameSettings>().initial_spawn.area;
        let deadline = Instant::now() + DRAWN_WITHIN;
        loop {
            let snapshot = self.snapshot(content);
            if snapshot.me.is_some() && snapshot.area == Some(area) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the player never arrived in the spawn area"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// The window as it shows now.
    fn look(&mut self, pictures: &Path) -> RgbaImage {
        let path = pictures.join("look.png");
        let answer = self.ask(&serde_json::json!({ "ask": "picture", "path": path }));
        assert_eq!(answer.trim(), "true", "the client pictured its window");
        image::open(&path).expect("a picture").to_rgba8()
    }

    /// Watches the window until it looks like `snapshot`, or the time runs out; answers how close
    /// it came, and the look that came closest.
    fn until_alike(&mut self, snapshot: &RgbaImage, pictures: &Path) -> (f32, RgbaImage) {
        let deadline = Instant::now() + DRAWN_WITHIN;
        let mut closest: Option<(f32, RgbaImage)> = None;
        while Instant::now() < deadline {
            let seen = self.look(pictures);
            let alike = resemblance(&seen, snapshot);
            if closest.as_ref().is_none_or(|(best, _)| alike >= *best) {
                closest = Some((alike, seen));
            }
            if alike >= LOOKS_ALIKE {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        closest.expect("a look at the window")
    }

    /// Watches the window until the picture holds still, as it does once the view's reveal has
    /// played.
    fn settled(&mut self, pictures: &Path) -> RgbaImage {
        let deadline = Instant::now() + DRAWN_WITHIN;
        let mut last = self.look(pictures);
        let mut still = 0;
        while still < SETTLED_FOR {
            assert!(
                Instant::now() < deadline,
                "the window never settled on a picture"
            );
            std::thread::sleep(Duration::from_secs(1));
            let seen = self.look(pictures);
            let held = resemblance(&seen, &last) >= STILL && !blank(&seen);
            still = if held { still + 1 } else { 0 };
            last = seen;
        }
        last
    }
}

/// A private X display for the client's window, gone when dropped.
struct Display {
    _server: Running,
    number: String,
}

impl Display {
    fn start() -> Display {
        let mut server = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0"])
            .arg(format!("{}x{}x24", SCREEN.0, SCREEN.1))
            .stdout(Stdio::piped())
            .spawn()
            .expect("Xvfb runs: the smoke test draws the client on a display of its own");
        let mut number = String::new();
        let mut stdout = server.stdout.take().expect("a piped stdout");
        let mut byte = [0];
        while stdout.read(&mut byte).is_ok_and(|read| read == 1) && byte[0] != b'\n' {
            number.push(char::from(byte[0]));
        }
        assert!(!number.is_empty(), "Xvfb named its display");
        Display {
            _server: Running(server),
            number,
        }
    }
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

fn game_content() -> Content {
    load_content(
        Arc::new(FilesystemFolder(game())),
        "smoke",
        engine::core::content::LoadPurpose::Play,
    )
    .unwrap_or_else(|report| panic!("the game loads: {report}"))
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

/// The desktop client in `folder`, as its player starts it there, but signed in as `token`,
/// drawing on `display` and answering the probe's driver at `driver`.
fn desktop_client(
    folder: &Path,
    token: &str,
    display: &Display,
    driver: std::net::SocketAddr,
) -> Running {
    let binary = folder.join("desktop-client");
    let probe = driver.to_string();
    Running(
        Command::new(&binary)
            .current_dir(folder)
            .env_remove("WAYLAND_DISPLAY")
            .env("DISPLAY", format!(":{}", display.number))
            .env("WGPU_ADAPTER_NAME", RASTERIZER)
            .env(format!("{CLIENT_ENV_PREFIX}ACCESS_TOKEN"), token)
            .env(format!("{CLIENT_ENV_PREFIX}PROBE"), probe)
            .env(format!("{CLIENT_ENV_PREFIX}EXIT_ON_STDIN_EOF"), "true")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("{} doesn't run: {error}", binary.display())),
    )
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
