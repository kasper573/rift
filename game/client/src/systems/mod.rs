//! The high-level client systems — the game itself, each plugging into the `crate::core` engines:
//! [`actor`] (sprites + the actor's own animation/footstep sounds), [`area`] tile rendering,
//! [`combat`] (the player's health bar + death tint), [`items`] (item-use sounds), [`view`] (the local
//! player's camera + audio listener), [`session`] (client session wiring + join/spectate announce),
//! [`input`] gestures (with the active-gesture tile highlight), the [`hud`], and [`debug`]/[`testing`].

pub mod actor;
pub mod area;
pub mod combat;
pub mod debug;
pub mod hud;
pub mod input;
pub mod items;
pub mod session;
pub mod testing;
pub mod view;
