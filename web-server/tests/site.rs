use std::collections::BTreeMap;

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

const ACCOUNT: &str = "ada";
const SITE: &str = "https://site.test";
const END_SESSION: &str = "https://provider.test/logout";
const ACCOUNT_PAGE: &str = "https://provider.test/account";
const GAME: &str = "Lantern Isle";
const RELEASE: &str = "2026.10.29-abcdef12";

#[tokio::test(flavor = "multi_thread")]
async fn a_visitor_signs_in_as_their_account_and_signs_out() {
    for account_page in [Some(ACCOUNT_PAGE), None] {
        let provider = start_provider().await;
        let site = site(&provider, account_page).await;
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
async fn the_downloads_offer_the_desktop_client_of_the_release_with_a_rift_env_for_the_site() {
    let provider = start_provider().await;
    let site = site(&provider, None).await;
    let mut visitor = Visitor::default();

    let downloads = visitor.get(&site, "/downloads").await;
    for archive in [
        "desktop-client-linux-x86_64.tar.gz",
        "desktop-client-windows-x86_64.zip",
        "desktop-client-macos-arm64.tar.gz",
    ] {
        let link = format!("https://releases.test/download/{RELEASE}/{archive}");
        assert!(downloads.body.contains(&link), "{link}");
    }
    assert!(!downloads.body.contains("editor"));

    let env = visitor.get(&site, "/rift.env").await;
    for line in [
        "RIFT_CLIENT_GAME_SERVER_URL=https://game-server.site.test",
        "RIFT_CLIENT_AUTH=oidc",
        &format!(
            "RIFT_AUTH_AUTHORIZATION_URI={}",
            provider.endpoints.authorize
        ),
        "RIFT_AUTH_SIGN_UP_URI=https://provider.test/sign-up",
        "RIFT_AUTH_TOKEN_URI=https://provider.test/token",
    ] {
        assert!(
            env.body.lines().any(|held| held == line),
            "{line}:\n{}",
            env.body
        );
    }
}

#[derive(Default)]
struct Visitor {
    cookies: BTreeMap<String, String>,
}

struct Page {
    status: StatusCode,
    location: Option<String>,
    body: String,
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
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|location| location.to_str().expect("a location").to_owned());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");
        Page {
            status,
            location,
            body: String::from_utf8(body.to_vec()).expect("a text body"),
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

async fn site(provider: &OAuthTestServer, account_page: Option<&str>) -> Router {
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
            game_server_ws_url: "wss://game-server.site.test".to_owned(),
            game_server_internal_url: "http://game-server:9998".to_owned(),
            downloads_url: "https://releases.test/download".to_owned(),
            release: RELEASE.to_owned(),
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
