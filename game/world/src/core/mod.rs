//! Game-agnostic substrate: geometry, time, tile space, pathfinding, and the content-table
//! machinery each feature's catalog is built on. Nothing here knows a specific game.

pub mod assets;
pub mod math;
pub mod nav;
pub mod table;
pub mod tiling;
pub mod time;
