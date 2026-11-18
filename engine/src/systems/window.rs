//! What every window a server opens for a player shares: the turn their requests answer, the
//! count that tells an answer from silence, and the rule that keeps the window open.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// The turn of a window a request answers, so a second press or a late packet answers a turn that
/// is already over.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowStep(pub u32);

/// Deals every window's turns.
#[derive(Resource, Default)]
pub struct WindowSteps(u32);

impl WindowSteps {
    pub fn deal(&mut self) -> WindowStep {
        self.0 = self.0.wrapping_add(1);
        WindowStep(self.0)
    }
}

/// How many requests a window has answered. A request that changes nothing else still raises it,
/// so whoever sent it can tell an answer from silence.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnswerCount(pub u32);

impl AnswerCount {
    pub fn raise(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

/// The state of a window a player has open: it stays open while the player can keep it, and until
/// they close it.
pub trait OpenWindow: Component {
    /// Whether `player` can still keep the window open.
    fn kept(&self, world: &World, player: Entity) -> bool;

    /// Closes the window of `player`, who asked to or can no longer keep it.
    fn close(world: &mut World, player: Entity);
}

/// Closes every window of kind `W` whose player can no longer keep it open.
pub fn hold<W: OpenWindow>(world: &mut World, windows: &mut QueryState<(Entity, &'static W)>) {
    let over: Vec<Entity> = windows
        .iter(world)
        .filter(|(player, window)| !window.kept(world, *player))
        .map(|(player, _)| player)
        .collect();
    for player in over {
        W::close(world, player);
    }
}
