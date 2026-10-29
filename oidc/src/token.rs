use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Where a service checks the provider's access tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCheckSettings {
    pub issuer: String,
    pub client_id: String,
    pub jwks_uri: String,
}

/// Checks access tokens against the provider's published keys, fetching them again when a token
/// names a key it doesn't know.
pub struct TokenVerifier {
    settings: TokenCheckSettings,
    agent: ureq::Agent,
    keys: Option<JwkSet>,
    last_fetch: Fetch,
}

impl TokenVerifier {
    pub fn new(settings: TokenCheckSettings) -> TokenVerifier {
        TokenVerifier {
            settings,
            agent: agent(),
            keys: None,
            last_fetch: Fetch::Never,
        }
    }

    pub fn issuer(&self) -> &str {
        &self.settings.issuer
    }

    pub fn warm(&mut self) -> Result<(), String> {
        self.fetch()
    }

    /// Whether it holds the provider's keys, fetching them if it may.
    pub fn ready(&mut self) -> bool {
        if self.keys.is_none() && self.cooldown_passed() {
            let _ = self.fetch();
        }
        self.keys.is_some()
    }

    /// The token's claims, if the provider signed it for this client and it hasn't expired. A token
    /// is for the client when its audience holds the client or the client is its authorized party,
    /// as providers differ in which of the two they set.
    pub fn verify<Claims: DeserializeOwned>(&mut self, token: &str) -> Result<Claims, String> {
        let header = decode_header(token).map_err(|error| error.to_string())?;
        if header.alg != Algorithm::RS256 {
            return Err(format!("unsupported algorithm {:?}", header.alg));
        }
        let kid = header.kid.ok_or("token has no key id")?;
        let key = self.key(&kid)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.settings.issuer]);
        validation.set_required_spec_claims(&["exp", "iss"]);
        validation.validate_aud = false;
        let checked = decode::<Checked<Claims>>(token, &key, &validation)
            .map_err(|error| error.to_string())?
            .claims;
        let client = self.settings.client_id.as_str();
        if !checked.aud.holds(client) && checked.azp.as_deref() != Some(client) {
            return Err("token is for another client".to_owned());
        }
        Ok(checked.claims)
    }

    fn key(&mut self, kid: &str) -> Result<DecodingKey, String> {
        let known =
            |keys: &Option<JwkSet>| keys.as_ref().is_some_and(|set| set.find(kid).is_some());
        if !known(&self.keys) && self.cooldown_passed() {
            self.fetch()?;
        }
        let jwk = self
            .keys
            .as_ref()
            .and_then(|set| set.find(kid))
            .ok_or("token signed by unknown key")?;
        DecodingKey::from_jwk(jwk).map_err(|error| error.to_string())
    }

    fn cooldown_passed(&self) -> bool {
        match self.last_fetch {
            Fetch::Never => true,
            Fetch::Failed(at) => at.elapsed() >= Duration::from_secs(1),
            Fetch::Succeeded(at) => at.elapsed() >= Duration::from_secs(30),
        }
    }

    fn fetch(&mut self) -> Result<(), String> {
        let result = self.try_fetch();
        self.last_fetch = match result {
            Ok(()) => Fetch::Succeeded(Instant::now()),
            Err(_) => Fetch::Failed(Instant::now()),
        };
        result
    }

    fn try_fetch(&mut self) -> Result<(), String> {
        let mut response = self
            .agent
            .get(&self.settings.jwks_uri)
            .call()
            .map_err(|error| format!("jwks fetch failed: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("jwks read failed: {error}"))?;
        self.keys =
            Some(serde_json::from_str(&body).map_err(|error| format!("jwks invalid: {error}"))?);
        Ok(())
    }
}

enum Fetch {
    Never,
    Failed(Instant),
    Succeeded(Instant),
}

#[derive(Deserialize)]
struct Checked<Claims> {
    #[serde(default)]
    aud: Audience,
    #[serde(default)]
    azp: Option<String>,
    #[serde(flatten)]
    claims: Claims,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Audience {
    #[default]
    Nobody,
    One(String),
    Several(Vec<String>),
}

impl Audience {
    fn holds(&self, client: &str) -> bool {
        match self {
            Audience::Nobody => false,
            Audience::One(audience) => audience == client,
            Audience::Several(audiences) => audiences.iter().any(|audience| audience == client),
        }
    }
}

// TLS against the OS's trust store, so a locally trusted authority works like any public one.
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build();
    ureq::Agent::new_with_config(config)
}
