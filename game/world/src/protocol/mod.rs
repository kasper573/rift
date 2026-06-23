//! The wire protocol: the replicated [`components`] both sides share, the [`messages`] they exchange,
//! and the registration that binds them to replication channels. [`session`]/[`query`] are the
//! client's read/act view over this state; [`channels`] is the renet transport-channel bridge.

pub mod channels;
pub mod components;
pub mod messages;
pub mod query;
pub mod session;

pub use components::*;
pub use messages::*;

use bevy_app::App;

pub fn protocol(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Position>()
        .replicate::<Actor>()
        .replicate::<Hitbox>()
        .replicate::<Vitals>()
        .replicate::<AreaTag>()
        .replicate::<Owner>()
        .replicate::<Name>()
        .replicate::<Spectate>()
        .replicate::<Xp>()
        .replicate::<Inventory>()
        .add_client_message::<JoinRequest>(Channel::Ordered)
        .add_client_message::<RespawnRequest>(Channel::Ordered)
        .add_client_message::<MoveRequest>(Channel::Ordered)
        .add_client_message::<MoveToPortal>(Channel::Ordered)
        .add_mapped_client_message::<AttackRequest>(Channel::Ordered)
        .add_client_message::<UseItemRequest>(Channel::Ordered)
        .add_client_message::<SpectateRequest>(Channel::Ordered)
        .add_server_message::<Welcome>(Channel::Ordered)
        .add_mapped_server_message::<ItemConsumed>(Channel::Ordered);
}
