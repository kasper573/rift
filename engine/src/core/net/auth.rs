use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone)]
pub struct Credentials {
    pub authorization: String,
}

impl Credentials {
    pub fn from_access_token(access_token: &str) -> Credentials {
        Credentials {
            authorization: format!("Bearer {access_token}"),
        }
    }
}

/// The claims of an access token the game reads, shaped as the identity provider issues them.
#[derive(Serialize, Deserialize, Default)]
pub struct TokenClaims {
    #[serde(default)]
    pub sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Every other claim, whose shape is each provider's own.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

/// What a claim lists: its values one by one, or in one string with spaces between them, as
/// providers write either.
#[derive(Deserialize)]
#[serde(untagged)]
enum ListedClaim {
    Each(Vec<String>),
    Spaced(String),
}

impl TokenClaims {
    /// What the game calls the account: OIDC guarantees only `sub`, so the friendlier claims come
    /// first where the provider sets them.
    pub fn display_name(&self) -> String {
        self.preferred_username
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.sub.clone())
    }

    /// What the claim at `pointer`, a JSON pointer into the token's claims, lists; nothing when
    /// the token has no such claim.
    pub fn listed(&self, pointer: &str) -> Vec<String> {
        let claims = serde_json::Value::Object(self.other.clone());
        let listed = claims
            .pointer(pointer)
            .and_then(|claim| ListedClaim::deserialize(claim).ok());
        match listed {
            Some(ListedClaim::Each(values)) => values,
            Some(ListedClaim::Spaced(values)) => {
                values.split_whitespace().map(str::to_owned).collect()
            }
            None => Vec::new(),
        }
    }

    // Without verifying the signature: the server verifies the token independently on every
    // session request.
    pub fn read_unverified(token: &str) -> Option<TokenClaims> {
        let payload = token.split('.').nth(1)?;
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
    }

    /// A token without a signature, which only a server that trusts every token takes.
    pub fn unsigned(&self) -> String {
        let claims = serde_json::to_vec(self).expect("serializable claims");
        format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode("{}"),
            URL_SAFE_NO_PAD.encode(claims)
        )
    }
}
