use bevy::prelude::Resource;

/// The signed-in player's session, built from the access token the website injects into the page. The
/// client does no auth itself: it just presents this token to the game server when opening a
/// connection. Decoding the token's roles is a game concern and lives in `crate::systems::account`.
#[derive(Resource, Clone)]
pub struct Session {
    pub authorization: String,
}

impl Session {
    pub fn from_access_token(access_token: &str) -> Session {
        Session {
            authorization: format!("Bearer {access_token}"),
        }
    }
}
