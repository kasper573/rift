//! The MMORPG game library: the **isomorphic** core both the server and the client share — the
//! replicated components, the client⇄server messages and their registration, the static content, and
//! the game-agnostic substrate ([`core`]). Organized by feature ([`actor`], [`movement`], [`combat`],
//! [`items`], [`player`], [`spectate`], [`area`], [`sfx`]). The authoritative simulation lives in the
//! `server` crate; the client's view of this world lives in `client::session`.

pub mod account;
pub mod actor;
pub mod area;
pub mod channels;
pub mod combat;
pub mod core;
pub mod items;
pub mod movement;
pub mod player;
pub mod sfx;
pub mod spectate;

use bevy_app::App;

pub const TICK_HZ: core::time::Hertz = core::time::Hertz(30.0);

/// Registers every feature's replicated components and client⇄server messages. Both the client
/// session and the server app call this so the two sides agree on the wire.
pub fn protocol(app: &mut App) {
    actor::register(app);
    area::register(app);
    combat::register(app);
    items::register(app);
    movement::register(app);
    player::register(app);
    spectate::register(app);
}
