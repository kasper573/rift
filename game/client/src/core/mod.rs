//! Low-level client infrastructure and primitives the game systems build on: the platform adapter,
//! the embedded asset source, the render pipeline, the netcode transport, the audio engine, and the
//! debug/test tooling. The high-level game systems live under `crate::systems`.

pub mod assets;
pub mod audio;
pub mod debug;
pub mod net;
pub mod platform;
pub mod render;
pub mod testing;
