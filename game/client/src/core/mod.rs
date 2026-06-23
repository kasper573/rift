//! Generic, not-game-specific client infrastructure any feature may build on: the platform adapter,
//! the embedded asset source, the render pipeline, the netcode transport, and the audio engine.
//! Anything bespoke to this game lives under `crate::systems`.

pub mod assets;
pub mod audio;
pub mod net;
pub mod platform;
pub mod render;
