//! The high-level client systems — the game itself, each plugging into the `crate::core` engines:
//! [`actor`] (sprites + the actor's own animation/footstep sounds), [`area`] tile rendering,
//! [`combat`] (the player's health bar + death tint), [`item`] (item-use sounds + dropped-item
//! rendering), [`view`] (the local
//! player's camera + audio listener), [`session`] (client session wiring + join/spectate announce),
//! [`input`] gestures (with the active-gesture tile highlight), the [`hud`], and [`debug`]/[`testing`].

pub mod actor;
pub mod area;
pub mod combat;
pub mod debug;
pub mod hud;
pub mod input;
pub mod item;
pub mod session;
pub mod testing;
pub mod view;

use bevy::prelude::*;

/// Despawns every entity carrying marker component `M` — the shared teardown a scene or overlay root
/// registers on an `OnExit`/state change, monomorphized per marker at the call site.
pub(crate) fn despawn_all<M: Component>(entities: Query<Entity, With<M>>, mut commands: Commands) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}
