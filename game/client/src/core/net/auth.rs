use bevy::prelude::Resource;

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
