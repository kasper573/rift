use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};
use sha2::{Digest, Sha256};

use crate::App;

pub async fn sign_in(
    State(app): State<Arc<App>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let return_path = sanitize_return(params.get("return"));
    let state = random_hex(16);
    let verifier = random_hex(32);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    let authorize = format!(
        "{}/protocol/openid-connect/auth?client_id={}&redirect_uri={}&response_type=code\
         &scope=openid%20profile&state={state}&code_challenge={challenge}&code_challenge_method=S256",
        app.authority,
        urlencode(&app.audience),
        urlencode(&app.redirect_uri),
    );
    (
        AppendHeaders([set_cookie(&format!(
            "oidc={state}.{verifier}.{return_path}; Path=/; Max-Age=600; HttpOnly; Secure; SameSite=Lax"
        ))]),
        Redirect::to(&authorize),
    )
        .into_response()
}

pub async fn callback(
    State(app): State<Arc<App>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some((state, verifier, return_path)) =
        cookie(&headers, "oidc").as_deref().and_then(|value| {
            let mut parts = value.splitn(3, '.');
            Some((
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
            ))
        })
    else {
        return Redirect::to("/").into_response();
    };
    if params.get("state") != Some(&state) {
        return Redirect::to("/").into_response();
    }
    let Some(code) = params.get("code").cloned() else {
        return Redirect::to("/").into_response();
    };

    let exchange = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || exchange_code(&app, &code, &verifier)).await
    };
    match exchange {
        Ok(Some((token, id_token))) => (
            AppendHeaders([
                set_cookie(&format!(
                    "token={token}; Path=/; HttpOnly; Secure; SameSite=Lax"
                )),
                set_cookie(&format!(
                    "idt={id_token}; Path=/; HttpOnly; Secure; SameSite=Lax"
                )),
                set_cookie("oidc=; Path=/; Max-Age=0"),
            ]),
            Redirect::to(&return_path),
        )
            .into_response(),
        _ => Redirect::to("/").into_response(),
    }
}

pub async fn sign_out(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let mut logout = format!(
        "{}/protocol/openid-connect/logout?client_id={}&post_logout_redirect_uri={}",
        app.authority,
        urlencode(&app.audience),
        urlencode(origin(&app.redirect_uri)),
    );
    // Without the hint keycloak interposes a logout confirmation page.
    if let Some(id_token) = cookie(&headers, "idt") {
        logout.push_str("&id_token_hint=");
        logout.push_str(&urlencode(&id_token));
    }
    (
        AppendHeaders([
            set_cookie("token=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax"),
            set_cookie("idt=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax"),
        ]),
        Redirect::to(&logout),
    )
        .into_response()
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split("; ")
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
        .map(str::to_owned)
}

pub fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn exchange_code(app: &App, code: &str, verifier: &str) -> Option<(String, String)> {
    let mut response = app
        .http
        .post(&app.token_uri)
        .send_form([
            ("grant_type", "authorization_code"),
            ("client_id", &app.audience),
            ("redirect_uri", &app.redirect_uri),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .ok()?;
    let body: serde_json::Value =
        serde_json::from_str(&response.body_mut().read_to_string().ok()?).ok()?;
    let token = body["access_token"].as_str()?.to_owned();
    let id_token = body["id_token"].as_str()?.to_owned();
    app.verifier.lock().ok()?.verify(&token).ok()?;
    Some((token, id_token))
}

// Browsers normalize `\` to `/` in URLs, so `/\host` would redirect off-site.
fn sanitize_return(raw: Option<&String>) -> String {
    match raw.map(String::as_str) {
        Some(path) if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') => {
            path.to_owned()
        }
        _ => "/".to_owned(),
    }
}

fn origin(url: &str) -> &str {
    match url.find("://") {
        Some(scheme) => match url[scheme + 3..].find('/') {
            Some(path) => &url[..scheme + 3 + path],
            None => url,
        },
        None => url,
    }
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    let mut file = std::fs::File::open("/dev/urandom").expect("/dev/urandom");
    std::io::Read::read_exact(&mut file, &mut buffer).expect("read randomness");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let buffer = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = u32::from_be_bytes([0, buffer[0], buffer[1], buffer[2]]);
        for position in 0..=chunk.len() {
            out.push(ALPHABET[(value >> (18 - 6 * position) & 0x3F) as usize] as char);
        }
    }
    out
}

fn set_cookie(value: &str) -> (header::HeaderName, String) {
    (header::SET_COOKIE, value.to_owned())
}
