use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claims {
    pub subject: String,
    pub name: String,
    pub roles: Vec<String>,
}

/// Verifies access tokens against the issuer's remote JWK set, fetched lazily and refreshed when
/// an unknown key id appears (key rotation), rate-limited by a cooldown.
pub struct Verifier {
    issuer: String,
    audience: String,
    jwks_uri: String,
    agent: ureq::Agent,
    keys: Option<JwkSet>,
    last_fetch: Fetch,
}

enum Fetch {
    Never,
    Failed(Instant),
    Succeeded(Instant),
}

impl Verifier {
    pub fn new(issuer: &str, audience: &str, jwks_uri: &str) -> Self {
        // TLS against the OS trust store (not rustls' baked-in roots), so a locally trusted CA
        // (the dev/test proxy's Caddy CA) works like any public one.
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Self {
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            jwks_uri: jwks_uri.to_owned(),
            agent: ureq::Agent::new_with_config(config),
            keys: None,
            last_fetch: Fetch::Never,
        }
    }

    /// Failure is not fatal: verification refetches on demand.
    pub fn warm(&mut self) -> Result<(), String> {
        self.fetch()
    }

    /// Whether tokens can be verified right now; fetches the keys if they are missing. Lets a
    /// health endpoint hold a freshly started server out of rotation until auth works.
    pub fn ready(&mut self) -> bool {
        if self.keys.is_none() && self.cooldown_passed() {
            let _ = self.fetch();
        }
        self.keys.is_some()
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
        let subject = claim("sub").ok_or("token has no subject")?;
        let name = claim("preferred_username").unwrap_or_else(|| subject.clone());
        let roles = data.claims["realm_access"]["roles"]
            .as_array()
            .map(|roles| {
                roles
                    .iter()
                    .filter_map(|role| role.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Claims {
            subject,
            name,
            roles,
        })
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

    /// Unknown-key refetches are rate-limited to keep bad tokens from hammering the issuer, but
    /// a failed fetch retries quickly: a brief issuer outage must not lock players out after it.
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
