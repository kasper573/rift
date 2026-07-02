use base64::Engine;
use bevy::prelude::Resource;
use serde::Deserialize;

#[derive(Resource, Clone)]
pub struct Session {
    pub authorization: String,
    pub roles: Vec<String>,
}

impl Session {
    pub fn from_access_token(access_token: &str) -> Session {
        Session {
            authorization: format!("Bearer {access_token}"),
            roles: roles(access_token),
        }
    }
}

// Claims are read without verifying the signature: roles only gate client-side UI,
// and the server verifies the token independently on every session request.
#[derive(Deserialize, Default)]
struct Claims {
    #[serde(default)]
    realm_access: RealmAccess,
}

#[derive(Deserialize, Default)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

fn roles(access_token: &str) -> Vec<String> {
    claims(access_token).unwrap_or_default().realm_access.roles
}

fn claims(access_token: &str) -> Option<Claims> {
    let payload = access_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
