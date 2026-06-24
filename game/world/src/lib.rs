//! The MMORPG game, headless. [`core`] is the game-agnostic substrate (geometry, time, tile space,
//! pathfinding, content tables, the netcode channel bridge); [`systems`] *is* the game — every
//! gameplay feature plus the headless [`systems::server_app`] that composes them. The `client` crate
//! wraps a frontend (rendering/input) around this and the `server` crate a backend (netcode/HTTP); a
//! benchmark consumes it standalone to measure the headless tick.

pub mod core;
pub mod systems;
