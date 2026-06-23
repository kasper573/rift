//! The game-facing client, by feature: per-feature rendering ([`actor`], [`area`]) and the interactive
//! [`input`] (gestures) and [`hud`] (panes, scenes) subsystems, built on the `crate::core` pipeline.

pub mod actor;
pub mod area;
pub mod hud;
pub mod input;
