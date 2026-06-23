//! Spectating: the replicated [`Spectate`] anchor that follows a watched player and the request to
//! start or change who's watched. The server systems that drive anchors live in the `server` crate.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::player::ClientId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Spectate>()
        .add_client_message::<SpectateRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Spectate {
    pub watch: Option<ClientId>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpectateRequest {
    pub watch: Option<ClientId>,
}
