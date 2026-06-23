use base64::Engine;
use bevy::prelude::Resource;
use world::account::Role;

/// The signed-in player's session, built from the access token the website injects into the page.
/// The client does no auth itself: it decodes the token's roles and presents the token to the game
/// server when opening a connection.
#[derive(Resource, Clone)]
pub struct Session {
    pub authorization: String,
    pub roles: Vec<Role>,
}

impl Session {
    pub fn from_access_token(access_token: &str) -> Session {
        Session {
            authorization: format!("Bearer {access_token}"),
            roles: roles(access_token),
        }
    }
}

fn roles(access_token: &str) -> Vec<Role> {
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
