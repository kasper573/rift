//! The high-level client systems — the game itself: per-feature rendering ([`actor`], [`area`]),
//! the world/screen [`overlay`]s (health bar, tile highlight, death wash), and the [`input`] gestures
//! and [`hud`]. All built on the `crate::core` primitives.

pub mod actor;
pub mod area;
pub mod hud;
pub mod input;
pub mod overlay;
