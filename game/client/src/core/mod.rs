//! Low-level, game-agnostic client infrastructure the systems build on: the platform adapter, the
//! embedded asset source, the render pipeline, the netcode transport, and the spatial audio mixer.
//! Nothing here depends on a systems layer (its own or `world`'s); the game systems plug in.

pub mod assets;
pub mod audio;
pub mod net;
pub mod platform;
pub mod render;
