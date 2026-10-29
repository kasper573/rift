use std::net::Ipv4Addr;
use std::sync::Arc;

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, HttpRequest, HttpResponse, PkceCodeChallenge,
    RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Response, Server};

/// Where a native app sends its user to sign in, and where it trades the answer for a token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSignInSettings {
    pub client_id: String,
    pub authorization_uri: String,
    /// A page that takes the same authorization request as `authorization_uri`, but asks for a
    /// new account.
    pub sign_up_uri: String,
    pub token_uri: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationPage {
    SignIn,
    SignUp,
}

/// Signs in in the system browser, which hands the sign-in back to a port of this machine's own.
pub struct BrowserSignIn {
    settings: BrowserSignInSettings,
    agent: ureq::Agent,
}

impl BrowserSignIn {
    pub fn new(settings: BrowserSignInSettings, agent: ureq::Agent) -> BrowserSignIn {
        BrowserSignIn { settings, agent }
    }

    /// Opens the authorization request at `page` with `open_browser`, and waits for the browser to
    /// come back with the answer.
    pub fn begin(
        &self,
        page: AuthorizationPage,
        open_browser: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<PendingSignIn, String> {
        let callback = Server::http((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("Couldn't listen for the sign-in: {error}"))?;
        let port = callback
            .server_addr()
            .to_ip()
            .expect("a tcp listener")
            .port();
        let page_uri = match page {
            AuthorizationPage::SignIn => &self.settings.authorization_uri,
            AuthorizationPage::SignUp => &self.settings.sign_up_uri,
        };
        let client = BasicClient::new(ClientId::new(self.settings.client_id.clone()))
            .set_auth_uri(AuthUrl::new(page_uri.clone()).map_err(invalid)?)
            .set_token_uri(TokenUrl::new(self.settings.token_uri.clone()).map_err(invalid)?)
            .set_redirect_uri(
                RedirectUrl::new(format!("http://{}:{port}{CALLBACK}", Ipv4Addr::LOCALHOST))
                    .map_err(invalid)?,
            );
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state) = client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("openid".to_owned()))
            .add_scope(Scope::new("profile".to_owned()))
            .set_pkce_challenge(challenge)
            .url();
        open_browser(url.as_str())?;
        let agent = self.agent.clone();
        let exchange = move |code: String| {
            let token = client
                .exchange_code(AuthorizationCode::new(code))
                .set_pkce_verifier(verifier)
                .request(&|request: HttpRequest| send(&agent, request))
                .map_err(|error| format!("The sign-in didn't go through: {error}"))?;
            Ok(token.access_token().secret().clone())
        };
        Ok(PendingSignIn {
            callback: Arc::new(callback),
            state,
            exchange: Box::new(exchange),
        })
    }
}

/// A sign-in the browser hasn't come back from yet.
pub struct PendingSignIn {
    callback: Arc<Server>,
    state: CsrfToken,
    exchange: Box<dyn FnOnce(String) -> Result<String, String> + Send>,
}

impl PendingSignIn {
    /// Stops the wait once the returned value drops, as whoever asked for the sign-in gives up.
    pub fn hang_up(&self) -> SignInHangUp {
        SignInHangUp(self.callback.clone())
    }

    /// Blocks until the browser comes back, and answers with the access token.
    pub fn finish(self) -> Result<String, String> {
        loop {
            let request = self
                .callback
                .recv()
                .map_err(|error| format!("The sign-in stopped: {error}"))?;
            let answer = request
                .url()
                .strip_prefix(CALLBACK)
                .and_then(|query| query.strip_prefix('?'))
                .and_then(|query| serde_urlencoded::from_str::<Answer>(query).ok())
                .filter(|answer| answer.state.as_deref() == Some(self.state.secret()));
            let Some(answer) = answer else {
                let _ = request.respond(Response::empty(404));
                continue;
            };
            let outcome = match answer.code {
                Some(code) => (self.exchange)(code),
                None => Err(answer
                    .error_description
                    .or(answer.error)
                    .unwrap_or_else(|| "The sign-in didn't finish.".to_owned())),
            };
            let page = match outcome {
                Ok(_) => SIGNED_IN_PAGE,
                Err(_) => NOT_SIGNED_IN_PAGE,
            };
            let _ = request.respond(Response::from_string(page).with_header(html()));
            return outcome;
        }
    }
}

pub struct SignInHangUp(Arc<Server>);

impl Drop for SignInHangUp {
    fn drop(&mut self) {
        self.0.unblock();
    }
}

const CALLBACK: &str = "/callback";

#[derive(Deserialize)]
struct Answer {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn send(agent: &ureq::Agent, request: HttpRequest) -> Result<HttpResponse, ureq::Error> {
    let (parts, mut body) = agent.run(request)?.into_parts();
    Ok(HttpResponse::from_parts(parts, body.read_to_vec()?))
}

fn invalid(error: oauth2::url::ParseError) -> String {
    format!("The sign-in settings hold a bad address: {error}")
}

fn html() -> Header {
    Header::from_bytes("Content-Type", "text/html; charset=utf-8").expect("a valid header")
}

const SIGNED_IN_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title>\
<p>You're signed in. Close this tab and return to the game.</p>";

const NOT_SIGNED_IN_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Not signed in</title>\
<p>The sign-in didn't finish. Return to the game to try again.</p>";
