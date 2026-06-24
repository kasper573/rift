//! The high-level client systems — the game itself, plugging into the `crate::core` primitives:
//! [`actor`] presentation (sprites, sound cues, the player camera/listener), [`area`] tile rendering,
//! world/screen [`overlay`]s (health bar, tile highlight, death wash), [`input`] gestures, the [`hud`]
//! (with the session announce in `hud::connection`), and [`debug`]/[`testing`] tooling.

pub mod actor;
pub mod area;
pub mod debug;
pub mod hud;
pub mod input;
pub mod overlay;
pub mod testing;
