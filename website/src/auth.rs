use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreGenderClaim, CoreJsonWebKeySet,
    CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
};
use openidconnect::{
    AdditionalClaims, AuthUrl, AuthorizationCode, Client, ClientId, CsrfToken, EndSessionUrl,
    EndpointNotSet, EndpointSet, IdToken, IdTokenClaims, IdTokenVerifier, IssuerUrl,
    JsonWebKeySetUrl, LogoutRequest, Nonce, NonceVerifier, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, PostLogoutRedirectUrl, RedirectUrl, Scope, TokenResponse, TokenUrl, reqwest,
};
use serde::{Deserialize, Serialize};

use crate::App;

pub struct Identity {
    pub name: String,
}

pub async fn identity(app: &App, jar: &CookieJar) -> Option<Identity> {
    let id_token: RiftIdToken = jar.get("idt")?.value().parse().ok()?;
    let claims = app.auth.verified_claims(&id_token, accept_nonce).await?;
    let name = claims
        .preferred_username()
        .map(|username| username.to_string())
        .unwrap_or_else(|| claims.subject().to_string());
    Some(Identity { name })
}

pub async fn sign_in(
    State(app): State<Arc<App>>,
    Query(params): Query<HashMap<String, String>>,
    jar: CookieJar,
) -> Response {
    let return_path = sanitize_return(params.get("return"));
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (authorize, state, nonce) = app
        .auth
        .client()
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("profile".to_owned()))
        .set_pkce_challenge(challenge)
        .url();
    let session = format!(
        "{}.{}.{}.{return_path}",
        state.secret(),
        nonce.secret(),
        verifier.secret()
    );
    (
        jar.add(cookie("oidc", &session)),
        Redirect::to(authorize.as_str()),
    )
        .into_response()
}

pub async fn callback(
    State(app): State<Arc<App>>,
    Query(params): Query<HashMap<String, String>>,
    jar: CookieJar,
) -> Response {
    let home = || Redirect::to("/").into_response();
    let Some((state, nonce, verifier, return_path)) =
        jar.get("oidc").map(Cookie::value).and_then(|value| {
            let mut parts = value.splitn(4, '.');
            Some((
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
            ))
        })
    else {
        return home();
    };
    if params.get("state").map(String::as_str) != Some(state.as_str()) {
        return home();
    }
    let Some(code) = params.get("code") else {
        return home();
    };

    let client = app.auth.client();
    let Ok(tokens) = client
        .exchange_code(AuthorizationCode::new(code.clone()))
        .set_pkce_verifier(PkceCodeVerifier::new(verifier))
        .request_async(&app.auth.http)
        .await
    else {
        return home();
    };
    let Some(id_token) = tokens.id_token().map(ToString::to_string) else {
        return home();
    };
    let Ok(parsed) = id_token.parse::<RiftIdToken>() else {
        return home();
    };
    if app
        .auth
        .verified_claims(&parsed, &Nonce::new(nonce))
        .await
        .is_none()
    {
        return home();
    }

    let jar = jar
        .add(cookie("token", tokens.access_token().secret()))
        .add(cookie("idt", &id_token))
        .remove(Cookie::build(("oidc", "")).path("/"));
    (jar, Redirect::to(&return_path)).into_response()
}

pub async fn sign_out(State(app): State<Arc<App>>, jar: CookieJar) -> Response {
    let mut logout = LogoutRequest::from(app.auth.end_session_url.clone())
        .set_client_id(app.auth.client_id.clone())
        .set_post_logout_redirect_uri(app.auth.post_logout.clone());
    let id_token: Option<RiftIdToken> = jar.get("idt").and_then(|idt| idt.value().parse().ok());
    if let Some(id_token) = &id_token {
        logout = logout.set_id_token_hint(id_token);
    }
    let jar = jar
        .remove(Cookie::build(("token", "")).path("/"))
        .remove(Cookie::build(("idt", "")).path("/"));
    (jar, Redirect::to(logout.http_get_url().as_str())).into_response()
}

pub struct Auth {
    client_id: ClientId,
    issuer: IssuerUrl,
    auth_url: AuthUrl,
    token_url: TokenUrl,
    redirect: RedirectUrl,
    end_session_url: EndSessionUrl,
    post_logout: PostLogoutRedirectUrl,
    jwks_url: JsonWebKeySetUrl,
    jwks: RwLock<CoreJsonWebKeySet>,
    last_jwks_fetch: Mutex<Option<Instant>>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct AuthConfig {
    issuer: IssuerUrl,
    audience: ClientId,
    jwks_uri: JsonWebKeySetUrl,
    token_uri: TokenUrl,
}

impl Auth {
    pub async fn from_env(redirect: RedirectUrl) -> Auth {
        let config: AuthConfig = envy::prefixed("RIFT_AUTH_")
            .from_env()
            .expect("RIFT_AUTH_* environment");
        let post_logout = PostLogoutRedirectUrl::new(redirect.url().origin().ascii_serialization())
            .expect("post-logout url");
        let auth = Auth {
            client_id: config.audience,
            auth_url: AuthUrl::new(format!(
                "{}/protocol/openid-connect/auth",
                config.issuer.as_str()
            ))
            .expect("auth url"),
            token_url: config.token_uri,
            end_session_url: EndSessionUrl::new(format!(
                "{}/protocol/openid-connect/logout",
                config.issuer.as_str()
            ))
            .expect("end-session url"),
            jwks_url: config.jwks_uri,
            jwks: RwLock::new(CoreJsonWebKeySet::new(Vec::new())),
            last_jwks_fetch: Mutex::new(None),
            http: reqwest::ClientBuilder::new()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .expect("http client"),
            issuer: config.issuer,
            redirect,
            post_logout,
        };
        match auth.refresh_jwks().await {
            Some(()) => println!("auth ready, issuer {}", auth.issuer.as_str()),
            None => println!(
                "auth ready, issuer {} (jwks warm-up failed)",
                auth.issuer.as_str()
            ),
        }
        auth
    }

    pub fn account_url(&self) -> String {
        format!("{}/account", self.issuer.as_str())
    }

    fn client(&self) -> RiftClient {
        Client::new(
            self.client_id.clone(),
            self.issuer.clone(),
            self.jwks
                .read()
                .map(|jwks| jwks.clone())
                .unwrap_or_default(),
        )
        .set_auth_uri(self.auth_url.clone())
        .set_token_uri(self.token_url.clone())
        .set_redirect_uri(self.redirect.clone())
    }

    async fn verified_claims<N: NonceVerifier + Copy>(
        &self,
        id_token: &RiftIdToken,
        nonce: N,
    ) -> Option<RiftClaims> {
        if let Some(claims) = self.try_claims(id_token, nonce) {
            return Some(claims);
        }
        self.refresh_jwks().await?;
        self.try_claims(id_token, nonce)
    }

    fn try_claims<N: NonceVerifier>(&self, id_token: &RiftIdToken, nonce: N) -> Option<RiftClaims> {
        let jwks = self.jwks.read().ok()?.clone();
        let verifier =
            IdTokenVerifier::new_public_client(self.client_id.clone(), self.issuer.clone(), jwks);
        id_token.claims(&verifier, nonce).ok().cloned()
    }

    async fn refresh_jwks(&self) -> Option<()> {
        const COOLDOWN: Duration = Duration::from_secs(30);
        {
            let mut last = self.last_jwks_fetch.lock().ok()?;
            let warm = !self.jwks.read().ok()?.keys().is_empty();
            if warm && last.is_some_and(|at| at.elapsed() < COOLDOWN) {
                return None;
            }
            *last = Some(Instant::now());
        }
        let fresh = CoreJsonWebKeySet::fetch_async(&self.jwks_url, &self.http)
            .await
            .ok()?;
        *self.jwks.write().ok()? = fresh;
        Some(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RealmClaims {
    #[serde(default)]
    realm_access: RealmRoles,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RealmRoles {
    #[serde(default)]
    roles: Vec<String>,
}

impl AdditionalClaims for RealmClaims {}

type RiftIdToken = IdToken<
    RealmClaims,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;

type RiftClaims = IdTokenClaims<RealmClaims, CoreGenderClaim>;

type RiftClient =
    CoreClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

fn accept_nonce(_: Option<&Nonce>) -> Result<(), String> {
    Ok(())
}

fn cookie(name: &'static str, value: &str) -> Cookie<'static> {
    Cookie::build((name, value.to_owned()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .build()
}

fn sanitize_return(raw: Option<&String>) -> String {
    match raw.map(String::as_str) {
        Some(path) if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') => {
            path.to_owned()
        }
        _ => "/".to_owned(),
    }
}
