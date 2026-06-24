//! Decodes the player's roles from the access token's JWT claims. Client-side and presentation-only
//! (it just decides whether to offer the spectate choice) — the server remains the source of truth.

use base64::Engine;
use world::systems::account::Role;

pub fn roles(access_token: &str) -> Vec<Role> {
    let Some(payload) = access_token.split('.').nth(1) else {
        return Vec::new();
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return Vec::new();
    };
    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    claims["realm_access"]["roles"]
        .as_array()
        .map(|roles| {
            roles
                .iter()
                .filter_map(|role| role.as_str().and_then(Role::parse))
                .collect()
        })
        .unwrap_or_default()
}
