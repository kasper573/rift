//! Client infrastructure shared by every feature: the platform adapter, the embedded asset source,
//! the render pipeline, the netcode transport, the audio engine, and dev tooling. Knows nothing of any
//! specific game feature — those live under `crate::systems`.

pub mod assets;
pub mod audio;
pub mod debug;
pub mod net;
pub mod platform;
pub mod render;
pub mod testing;
