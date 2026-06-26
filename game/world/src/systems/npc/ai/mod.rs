//! NPC aggro/wander strategies: one per file implementing [`Behavior`], dispatched by [`AiKind`]
//! (`enum_dispatch`). An [`NpcDef`](super::NpcDef) names its strategy; adding one is a new file plus
//! an `AiKind` variant — `run_ai` never matches on a specific behavior.

mod aggressive;
mod defensive;
mod pacifist;
mod protective;

pub use aggressive::Aggressive;
pub use defensive::Defensive;
pub use pacifist::Pacifist;
pub use protective::Protective;

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Deserializer};

use crate::core::math::{Pos, Rng};
use crate::core::table::Id;
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::area::{AreaDef, AreaTag};
use crate::systems::combat::is_dead;
use crate::systems::movement::position;

#[enum_dispatch]
pub trait Ai {
    /// The id this strategy is named by in `npc_table.json`; equals the snake_case struct name.
    fn name(&self) -> &str;
    /// Whether the npc wanders when it has nothing to chase.
    fn wanders(&self, rng: &mut Rng) -> bool;
    /// The entity to engage, if any.
    fn target(&self, hunt: &Hunt) -> Option<Entity>;
}

/// Every npc strategy, one variant per behavior. `enum_dispatch` forwards [`Ai`] to the variant.
#[enum_dispatch(Ai)]
#[derive(Clone, Copy)]
pub enum AiKind {
    Pacifist(Pacifist),
    Defensive(Defensive),
    Aggressive(Aggressive),
    Protective(Protective),
}

impl AiKind {
    fn all() -> [AiKind; 4] {
        [
            Pacifist.into(),
            Defensive.into(),
            Aggressive.into(),
            Protective.into(),
        ]
    }
}

impl<'de> Deserialize<'de> for AiKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        AiKind::all()
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown ai '{name}'")))
    }
}

/// What a [`Behavior`] selects a target from: the npc, its surroundings, and the candidate lists.
pub struct Hunt<'a> {
    pub world: &'a World,
    pub players: &'a [Entity],
    pub by_group: &'a HashMap<u32, Vec<Entity>>,
    pub id: Entity,
    pub group: u32,
    pub at: Pos<Tiles>,
    pub area: Id<AreaDef>,
    pub aggro: Tiles,
}

impl Hunt<'_> {
    /// The nearest accepted, living, same-area candidate within aggro range.
    pub fn nearest(
        &self,
        candidates: &[Entity],
        accept: impl Fn(Entity) -> bool,
    ) -> Option<Entity> {
        let mut best: Option<(Entity, Tiles)> = None;
        for &candidate in candidates {
            if is_dead(self.world, candidate)
                || self.world.get::<AreaTag>(candidate).map(|t| t.area) != Some(self.area)
                || !accept(candidate)
            {
                continue;
            }
            if let Some(at) = position(self.world, candidate) {
                let distance = self.at.distance(at);
                if distance <= self.aggro && best.is_none_or(|(_, best)| distance < best) {
                    best = Some((candidate, distance));
                }
            }
        }
        best.map(|(entity, _)| entity)
    }
}
