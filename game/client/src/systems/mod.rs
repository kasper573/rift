//! The high-level client systems — the game itself, plugging into the `crate::core` primitives:
//! per-feature rendering ([`actor`], [`area`], [`camera`]), world/screen [`overlay`]s (health bar,
//! tile highlight, death wash), sound cues ([`audio`]), [`input`] gestures, the [`hud`], the session
//! [`net`] announce + [`account`] role decode, and [`debug`]/[`testing`] tooling.

pub mod account;
pub mod actor;
pub mod area;
pub mod audio;
pub mod camera;
pub mod debug;
pub mod hud;
pub mod input;
pub mod net;
pub mod overlay;
pub mod testing;
