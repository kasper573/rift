use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use oauth2_test_server::{IssuerConfig, OAuthTestServer};
use openidconnect::{
    AuthUrl, ClientId, EndSessionUrl, IssuerUrl, JsonWebKeySetUrl, RedirectUrl, TokenUrl,
};
use serde::Serialize;
use service::GameInfo;
use tower::ServiceExt;
use web_server::{SignInSettings, Site, WebServerConfig};

const ACCOUNT: &str = "ada-the-visitor";
const SITE: &str = "https://site.test";
const END_SESSION: &str = "https://provider.test/logout";
const ACCOUNT_PAGE: &str = "https://provider.test/account";
const GAME: &str = "Lantern Isle";
const RELEASE: &str = "2026.10.29-abcdef12";
const PREVIEW_VIDEO: &str = "a moving picture";

#[tokio::test(flavor = "multi_thread")]
async fn a_visitor_signs_in_as_their_account_and_signs_out() {
    for account_page in [Some(ACCOUNT_PAGE), None] {
        let provider = start_provider().await;
        let site = site(&provider, account_page, NO_RELEASES).await;
        let mut visitor = Visitor::default();

        let landing = visitor.get(&site, "/").await;
        assert!(landing.body.contains(&format!("<title>{GAME}</title>")));
        assert!(!landing.body.contains(ACCOUNT));

        let asked = visitor.get(&site, "/auth/sign-in?return=/downloads").await;
        assert_eq!(asked.status, StatusCode::SEE_OTHER);
        let authorization = asked.location();
        assert!(authorization.starts_with(&provider.endpoints.authorize));
        let back = provider
            .http
            .get(&authorization)
            .send()
            .await
            .expect("the provider answers");
        let back = back.headers()[header::LOCATION.as_str()]
            .to_str()
            .expect("a redirect back")
            .to_owned();
        let callback = back.strip_prefix(SITE).expect("back to the site");

        let signed_in = visitor.get(&site, callback).await;
        assert_eq!(signed_in.location(), "/downloads");

        let downloads = visitor.get(&site, "/downloads").await;
        assert!(downloads.body.contains(ACCOUNT), "{}", downloads.body);
        assert_eq!(
            downloads.body.contains(&format!("href=\"{ACCOUNT_PAGE}\"")),
            account_page.is_some()
        );

        let signed_out = visitor.get(&site, "/auth/sign-out").await;
        let logout = signed_out.location();
        assert!(logout.starts_with(END_SESSION), "{logout}");
        assert!(logout.contains("id_token_hint="));
        let after = visitor.get(&site, "/downloads").await;
        assert!(!after.body.contains(ACCOUNT));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_is_the_releases_game_client_in_a_folder_with_everything_it_needs_to_play_here()
{
    let provider = start_provider().await;
    let releases = start_releases().await;
    let site = site(&provider, None, &releases).await;
    let mut visitor = Visitor::default();

    let downloads = visitor.get(&site, "/downloads").await;
    for platform in ["linux-x86_64", "windows-x86_64", "macos-arm64"] {
        let link = format!("href=\"/download/{platform}\"");
        assert!(downloads.body.contains(&link), "{link}");
    }
    for unsaid in ["rift.env", "RIFT_", "editor"] {
        assert!(!downloads.body.contains(unsaid), "the page says {unsaid}");
    }

    let rift_env = [
        "RIFT_CLIENT_GAME_SERVER_URL=https://game-server.site.test".to_owned(),
        "RIFT_CLIENT_AUTH=oidc".to_owned(),
        format!("RIFT_CLIENT_RELEASES_URL={releases}"),
        format!(
            "RIFT_AUTH_AUTHORIZATION_URI={}",
            provider.endpoints.authorize
        ),
        "RIFT_AUTH_SIGN_UP_URI=https://provider.test/sign-up".to_owned(),
        "RIFT_AUTH_TOKEN_URI=https://provider.test/token".to_owned(),
    ];
    for (platform, file, client) in [
        (
            "linux-x86_64",
            "Lantern-Isle-linux-x86_64.tar.gz",
            "game-client",
        ),
        (
            "windows-x86_64",
            "Lantern-Isle-windows-x86_64.zip",
            "game-client.exe",
        ),
    ] {
        let download = visitor.get(&site, &format!("/download/{platform}")).await;
        assert_eq!(
            download.disposition.as_deref(),
            Some(format!("attachment; filename=\"{file}\"").as_str()),
            "{platform}"
        );
        let held = extracted(file, &download.bytes);
        assert_eq!(
            held.keys().collect::<Vec<_>>(),
            [
                &format!("Lantern-Isle/{client}"),
                &"Lantern-Isle/rift.env".to_owned()
            ],
            "{platform}: one folder, named after the game"
        );
        assert_eq!(
            held[&format!("Lantern-Isle/{client}")],
            release_client(platform),
            "{platform}: the release's own game client"
        );
        if file.ends_with(".tar.gz") {
            assert_eq!(
                mode_of(&download.bytes, &format!("Lantern-Isle/{client}")),
                0o755,
                "{platform}: the game client starts as extracted"
            );
        }
        let env = String::from_utf8(held["Lantern-Isle/rift.env"].clone()).expect("text");
        for line in &rift_env {
            assert!(env.lines().any(|held| held == line), "{line}:\n{env}");
        }
    }

    for (path, status) in [
        ("/download/amiga-68k", StatusCode::NOT_FOUND),
        ("/download/macos-arm64", StatusCode::BAD_GATEWAY),
    ] {
        assert_eq!(visitor.get(&site, path).await.status, status, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_landing_page_plays_the_preview_video_the_site_is_given() {
    std::fs::write(preview_video(), PREVIEW_VIDEO).expect("a preview video");
    let provider = start_provider().await;
    let site = site(&provider, None, NO_RELEASES).await;
    let mut visitor = Visitor::default();

    let landing = visitor.get(&site, "/").await;
    let (_, player) = landing.body.split_once("<video").expect("a video player");
    let (_, source) = player.split_once("src=\"").expect("a source");
    let (source, _) = source.split_once('"').expect("a source");

    let video = visitor.get(&site, source).await;
    assert_eq!(video.status, StatusCode::OK);
    assert_eq!(video.body, PREVIEW_VIDEO);
}

fn preview_video() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("preview.mp4")
}

#[derive(Default)]
struct Visitor {
    cookies: BTreeMap<String, String>,
}

struct Page {
    status: StatusCode,
    location: Option<String>,
    disposition: Option<String>,
    body: String,
    bytes: Vec<u8>,
}

impl Page {
    fn location(&self) -> String {
        self.location.clone().expect("a redirect")
    }
}

impl Visitor {
    async fn get(&mut self, site: &Router, path: &str) -> Page {
        let cookies: Vec<String> = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        let request = Request::get(path)
            .header(header::COOKIE, cookies.join("; "))
            .body(Body::empty())
            .expect("a request");
        let response = site
            .clone()
            .oneshot(request)
            .await
            .expect("the site answers");
        for set in response.headers().get_all(header::SET_COOKIE) {
            let set = set.to_str().expect("a cookie");
            let (pair, attributes) = set.split_once(';').unwrap_or((set, ""));
            let (name, value) = pair.split_once('=').expect("a named cookie");
            if value.is_empty() || attributes.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
        let status = response.status();
        let header_text = |name| {
            response
                .headers()
                .get(name)
                .map(|value: &header::HeaderValue| value.to_str().expect("text").to_owned())
        };
        let location = header_text(header::LOCATION);
        let disposition = header_text(header::CONTENT_DISPOSITION);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");
        Page {
            status,
            location,
            disposition,
            body: String::from_utf8_lossy(&body).into_owned(),
            bytes: body.to_vec(),
        }
    }
}

#[derive(Serialize)]
struct Registration {
    scope: &'static str,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: &'static str,
}

async fn start_provider() -> OAuthTestServer {
    OAuthTestServer::start_with_config(IssuerConfig {
        host: "127.0.0.1".to_owned(),
        default_user_id: ACCOUNT.to_owned(),
        ..IssuerConfig::default()
    })
    .await
}

async fn site(provider: &OAuthTestServer, account_page: Option<&str>, releases: &str) -> Router {
    let registration = Registration {
        scope: "openid profile",
        redirect_uris: vec![format!("{SITE}/auth-callback")],
        token_endpoint_auth_method: "none",
    };
    let metadata = serde_json::to_value(registration).expect("a registration");
    let client = provider.register_client(metadata).await.client_id;
    web_server::router(Site {
        config: WebServerConfig {
            bind: "127.0.0.1".parse().expect("an address"),
            port: 0,
            redirect_uri: RedirectUrl::new(format!("{SITE}/auth-callback")).expect("a url"),
            game_server_url: "https://game-server.site.test".to_owned(),
            game_server_internal_url: "http://game-server:9998".to_owned(),
            downloads_url: releases.to_owned(),
            release: RELEASE.to_owned(),
            preview_video: preview_video(),
        },
        sign_in: SignInSettings {
            issuer: IssuerUrl::new(provider.issuer().to_owned()).expect("a url"),
            client_id: ClientId::new(client.clone()),
            jwks_uri: JsonWebKeySetUrl::new(provider.endpoints.jwks.clone()).expect("a url"),
            token_uri: TokenUrl::new(provider.endpoints.token.clone()).expect("a url"),
            authorization_uri: AuthUrl::new(provider.endpoints.authorize.clone()).expect("a url"),
            end_session_uri: EndSessionUrl::new(END_SESSION.to_owned()).expect("a url"),
            account_uri: account_page.map(str::to_owned),
        },
        desktop_sign_in: oidc::BrowserSignInSettings {
            client_id: client,
            authorization_uri: provider.endpoints.authorize.clone(),
            sign_up_uri: "https://provider.test/sign-up".to_owned(),
            token_uri: "https://provider.test/token".to_owned(),
        },
        game: GameInfo {
            name: GAME.to_owned(),
        },
    })
    .await
}

const NO_RELEASES: &str = "http://127.0.0.1:9/download";

/// Where the release's archives are published, holding the game client of Linux and Windows.
async fn start_releases() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("a bound port");
    let mut releases = Router::new();
    for (platform, archive) in [
        (
            "linux-x86_64",
            tar_gz("game-client", &release_client("linux-x86_64")),
        ),
        (
            "windows-x86_64",
            zipped("game-client.exe", &release_client("windows-x86_64")),
        ),
    ] {
        let suffix = if platform.starts_with("windows") {
            "zip"
        } else {
            "tar.gz"
        };
        let route = format!("/download/{RELEASE}/game-client-{platform}.{suffix}");
        releases = releases.route(&route, axum::routing::get(move || async move { archive }));
    }
    tokio::spawn(async move { axum::serve(listener, releases).await });
    format!("http://{address}/download")
}

/// A game client with a run of zeros in it, which a binary may have and its file on disk holds as a
/// hole.
fn release_client(platform: &str) -> Vec<u8> {
    let mut client = format!("the game client for {platform}").into_bytes();
    client.resize(CLIENT_HOLE, 0);
    client.extend_from_slice(b"and its end");
    client
}

const CLIENT_HOLE: usize = 1 << 20;

/// `bytes` archived from a file on disk with its zeros left as a hole, so the archive may hold it
/// as a sparse entry.
fn tar_gz(name: &str, bytes: &[u8]) -> Vec<u8> {
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::PermissionsExt;
    let held = tempfile::NamedTempFile::new().expect("a file");
    let mut file = held.as_file();
    let start = bytes.iter().position(|byte| *byte == 0).expect("a hole");
    let end = bytes.iter().rposition(|byte| *byte == 0).expect("a hole") + 1;
    file.write_all(&bytes[..start]).expect("written");
    file.seek(SeekFrom::Start(end as u64))
        .expect("past the hole");
    file.write_all(&bytes[end..]).expect("written");
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
        .expect("executable");
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(gz);
    archive
        .append_path_with_name(held.path(), name)
        .expect("a tar entry");
    archive
        .into_inner()
        .and_then(|gz| gz.finish())
        .expect("a tar.gz")
}

fn zipped(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(name, zip::write::SimpleFileOptions::default())
        .expect("a zip entry");
    std::io::Write::write_all(&mut archive, bytes).expect("written");
    archive.finish().expect("a zip").into_inner()
}

/// Every file `archive` holds, by its path in it.
fn extracted(file: &str, archive: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut held = BTreeMap::new();
    if file.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).expect("a zip");
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).expect("an entry");
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).expect("read");
            held.insert(entry.name().expect("a name").into_owned(), bytes);
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries().expect("a tar") {
            let mut entry = entry.expect("an entry");
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).expect("read");
            let path = entry.path().expect("a path").display().to_string();
            held.insert(path, bytes);
        }
    }
    held
}

fn mode_of(archive: &[u8], path: &str) -> u32 {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    tar.entries()
        .expect("a tar")
        .map(|entry| entry.expect("an entry"))
        .find(|entry| entry.path().is_ok_and(|held| held.to_str() == Some(path)))
        .and_then(|entry| entry.header().mode().ok())
        .expect("the file")
}
