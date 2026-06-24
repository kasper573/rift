//! Game-agnostic substrate: geometry, time, tile space, pathfinding, the content-table machinery each
//! feature's catalog is built on, and the engine-agnostic netcode channel bridge. Nothing here knows
//! a specific game.

pub mod assets;
pub mod channels;
pub mod math;
pub mod nav;
pub mod table;
pub mod tiling;
pub mod time;
