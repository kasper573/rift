//! Access-token verification for signed-in pages, against the realm's remote JWK set. The set is
//! fetched lazily and refreshed when an unknown key id appears (key rotation), rate-limited by a
//! cooldown.

use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claims {
    pub name: String,
    pub roles: Vec<String>,
}

pub struct Verifier {
    issuer: String,
    audience: String,
    jwks_uri: String,
    agent: ureq::Agent,
    keys: Option<JwkSet>,
    last_fetch: Option<Instant>,
}

impl Verifier {
    pub fn new(issuer: &str, audience: &str, jwks_uri: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build();
        Self {
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            jwks_uri: jwks_uri.to_owned(),
            agent: ureq::Agent::new_with_config(config),
            keys: None,
            last_fetch: None,
        }
    }

    /// Failure is not fatal: verification refetches on demand.
    pub fn warm(&mut self) -> Result<(), String> {
        self.fetch()
    }

    pub fn verify(&mut self, token: &str) -> Result<Claims, String> {
        let header = decode_header(token).map_err(|error| error.to_string())?;
        if header.alg != Algorithm::RS256 {
            return Err(format!("unsupported algorithm {:?}", header.alg));
        }
        let kid = header.kid.ok_or("token has no key id")?;
        let key = self.key(&kid)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "iss"]);
        // Keycloak names the client in `azp`; `aud` is unreliable across client configurations.
        validation.validate_aud = false;
        let data = decode::<serde_json::Value>(token, &key, &validation)
            .map_err(|error| error.to_string())?;

        let claim = |name: &str| {
            data.claims
                .get(name)
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };
        if claim("azp").as_deref() != Some(self.audience.as_str()) {
            return Err("token authorized party mismatch".to_owned());
        }
        let name = claim("preferred_username")
            .or_else(|| claim("sub"))
            .ok_or("token has no subject")?;
        let roles = data.claims["realm_access"]["roles"]
            .as_array()
            .map(|roles| {
                roles
                    .iter()
                    .filter_map(|role| role.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Claims { name, roles })
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
        const COOLDOWN: Duration = Duration::from_secs(30);
        self.last_fetch.is_none_or(|at| at.elapsed() >= COOLDOWN)
    }

    fn fetch(&mut self) -> Result<(), String> {
        self.last_fetch = Some(Instant::now());
        let mut response = self
            .agent
            .get(&self.jwks_uri)
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
