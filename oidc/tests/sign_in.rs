use std::time::{SystemTime, UNIX_EPOCH};

use oauth2_test_server::{IssuerConfig, Keys, OAuthTestServer};
use oidc::{
    AuthorizationPage, BrowserSignIn, BrowserSignInSettings, TokenCheckSettings, TokenVerifier,
};
use provider_jwt::{Algorithm, Header};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

const ACCOUNT: &str = "ada";
const NOWHERE: &str = "http://127.0.0.1:9/authorize";

#[test]
fn a_native_app_signs_in_and_up_in_the_browser_for_a_token_the_game_server_takes() {
    for page in [AuthorizationPage::SignIn, AuthorizationPage::SignUp] {
        let runtime = Runtime::new().expect("a runtime for the provider");
        let provider = runtime.block_on(start_provider());
        let client = runtime.block_on(register(&provider));
        let provider_page = provider.endpoints.authorize.clone();
        let (sign_in_uri, sign_up_uri) = match page {
            AuthorizationPage::SignIn => (provider_page, NOWHERE.to_owned()),
            AuthorizationPage::SignUp => (NOWHERE.to_owned(), provider_page),
        };
        let settings = BrowserSignInSettings {
            client_id: client.clone(),
            authorization_uri: sign_in_uri,
            sign_up_uri,
            token_uri: provider.endpoints.token.clone(),
        };
        let state = provider.state().clone();
        let handle = runtime.handle().clone();
        let pending = BrowserSignIn::new(settings, ureq::agent())
            .begin(page, |url| {
                let redirect = query(url, "redirect_uri").expect("the request names its redirect");
                handle.block_on(async {
                    let mut registered = state
                        .store
                        .get_client(&client)
                        .await
                        .expect("a registered client");
                    registered.redirect_uris.push(redirect);
                    state.store.insert_client(registered).await;
                });
                let url = url.to_owned();
                std::thread::spawn(move || browse(&url));
                Ok(())
            })
            .expect("the sign-in begins");

        let token = pending.finish().expect("the sign-in finishes with a token");

        let mut verifier = TokenVerifier::new(token_check(&provider, &client));
        let account: Account = verifier.verify(&token).expect("the game server takes it");
        assert_eq!(account.sub, ACCOUNT, "{page:?}");
    }
}

#[test]
fn the_game_server_takes_only_tokens_the_provider_signed_for_it() {
    let runtime = Runtime::new().expect("a runtime for the provider");
    let provider = runtime.block_on(start_provider());
    let client = runtime.block_on(register(&provider));
    let keys = &provider.state().keys;
    let stranger = Keys::generate();
    let issuer = provider.issuer().to_owned();
    let claims = |aud: Audience, azp: Option<&str>| Claims {
        iss: issuer.clone(),
        sub: ACCOUNT.to_owned(),
        exp: now() + 300,
        aud,
        azp: azp.map(str::to_owned),
        preferred_username: Some("Ada".to_owned()),
    };
    let to_client = Audience::One(client.clone());
    let cases: Vec<(&str, String, bool)> = vec![
        (
            "for the client",
            sign(keys, &claims(to_client.clone(), None)),
            true,
        ),
        (
            "for another audience, authorized by the client",
            sign(
                keys,
                &claims(Audience::One("account".to_owned()), Some(&client)),
            ),
            true,
        ),
        (
            "for several audiences, the client among them",
            sign(
                keys,
                &claims(
                    Audience::Several(vec!["account".to_owned(), client.clone()]),
                    None,
                ),
            ),
            true,
        ),
        (
            "for another client",
            sign(
                keys,
                &claims(Audience::One("other".to_owned()), Some("other")),
            ),
            false,
        ),
        (
            "from another issuer",
            sign(
                keys,
                &Claims {
                    iss: "http://127.0.0.1:9/elsewhere".to_owned(),
                    ..claims(to_client.clone(), None)
                },
            ),
            false,
        ),
        (
            "expired",
            sign(
                keys,
                &Claims {
                    exp: now() - 300,
                    ..claims(to_client.clone(), None)
                },
            ),
            false,
        ),
        (
            "signed by a key the provider never published",
            sign(&stranger, &claims(to_client.clone(), None)),
            false,
        ),
    ];
    let mut verifier = TokenVerifier::new(token_check(&provider, &client));
    assert!(verifier.ready(), "the provider's keys are fetched");
    for (case, token, taken) in cases {
        let verified = verifier.verify::<Account>(&token);
        assert_eq!(verified.is_ok(), taken, "{case}: {verified:?}");
        if let Ok(account) = verified {
            assert_eq!(account.preferred_username.as_deref(), Some("Ada"), "{case}");
        }
    }
}

#[derive(Deserialize, Debug)]
struct Account {
    sub: String,
    preferred_username: Option<String>,
}

#[derive(Serialize, Clone)]
struct Claims {
    iss: String,
    sub: String,
    exp: u64,
    aud: Audience,
    #[serde(skip_serializing_if = "Option::is_none")]
    azp: Option<String>,
    preferred_username: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
enum Audience {
    One(String),
    Several(Vec<String>),
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

async fn register(provider: &OAuthTestServer) -> String {
    let registration = Registration {
        scope: "openid profile",
        redirect_uris: vec![NOWHERE.to_owned()],
        token_endpoint_auth_method: "none",
    };
    let metadata = serde_json::to_value(registration).expect("a registration");
    provider.register_client(metadata).await.client_id
}

fn token_check(provider: &OAuthTestServer, client: &str) -> TokenCheckSettings {
    TokenCheckSettings {
        issuer: provider.issuer().to_owned(),
        client_id: client.to_owned(),
        jwks_uri: provider.endpoints.jwks.clone(),
    }
}

fn sign(keys: &Keys, claims: &Claims) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());
    provider_jwt::encode(&header, claims, &keys.encoding).expect("a signed token")
}

/// What the system browser does with the authorization request: the provider answers it with a
/// redirect back to the app, which the browser follows.
fn browse(url: &str) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let answer = agent.get(url).call().expect("the provider answers");
    let back = answer
        .headers()
        .get("location")
        .and_then(|location| location.to_str().ok())
        .expect("the provider redirects back")
        .to_owned();
    agent.get(&back).call().expect("the app takes the answer");
}

fn query(url: &str, name: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    let pairs: Vec<(String, String)> = serde_urlencoded::from_str(query).ok()?;
    pairs
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a time after the epoch")
        .as_secs()
}
