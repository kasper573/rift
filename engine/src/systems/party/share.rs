//! What a party shares of a kill: its experience.

use bevy_ecs::prelude::*;

use super::{ExperienceSplit, SharedExperience};
use crate::systems::GameSettings;
use crate::systems::class;

/// Grants `sharers` what each gets of a kill worth `worth`, as the game's parties share it.
pub fn grant_xp(world: &mut World, sharers: &[Entity], worth: u32) {
    let rules = world.resource::<GameSettings>().party;
    let levels: Vec<u32> = sharers
        .iter()
        .map(|&sharer| class::level(world, sharer).max(1))
        .collect();
    let shares = match rules {
        Some(rules) => shares(&rules.experience, worth, &levels),
        None => vec![worth; sharers.len()],
    };
    for (&sharer, share) in sharers.iter().zip(shares) {
        class::grant_xp(world, sharer, share);
    }
}

/// What each of the members who share a kill worth `worth` gets of it, by their `levels`.
pub fn shares(rules: &SharedExperience, worth: u32, levels: &[u32]) -> Vec<u32> {
    let further = levels.len().saturating_sub(1) as f32;
    let whole = worth as f32 * (1.0 + rules.bonus / 100.0 * further);
    let all: u32 = levels.iter().sum();
    levels
        .iter()
        .map(|&level| {
            let part = match rules.split {
                ExperienceSplit::Even => whole / levels.len() as f32,
                ExperienceSplit::ByLevel => whole * level as f32 / all.max(1) as f32,
                ExperienceSplit::Whole => whole,
            };
            part.round() as u32
        })
        .collect()
}
