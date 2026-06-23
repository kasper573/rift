//! The bespoke, game-specific client: per-feature rendering ([`actor`], [`area`]), world/screen
//! [`overlay`]s (health bar, tile highlight, death wash), the [`input`] gestures and [`hud`], plus
//! [`debug`] visualisation and e2e [`testing`] hooks. All built on the generic `crate::core` pipeline.

pub mod actor;
pub mod area;
pub mod debug;
pub mod hud;
pub mod input;
pub mod overlay;
pub mod testing;
