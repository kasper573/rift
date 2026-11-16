//! What a party shares of a kill: its experience, and the credit quests count.

use bevy_ecs::prelude::*;

use super::{ExperienceSplit, SharedExperience, companions};
use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::class;
use crate::systems::movement::position;
use crate::systems::player::{Away, Players};
use crate::systems::stat;

/// The characters that share in a kill credited to `credited`, of a foe that fell at `at`:
/// `credited` alone, or with a party every member of it who stands in this world within the
/// game's range of where the foe fell, alive and with their player at them. Whoever made the
/// kill counts from however far they made it.
pub fn sharers(world: &World, credited: Entity, at: Option<Pos<Meters>>) -> Vec<Entity> {
    let Some(rules) = world.resource::<GameSettings>().party else {
        return vec![credited];
    };
    let players = world.resource::<Players>();
    let members: Vec<Entity> = companions(world, credited)
        .iter()
        .filter_map(|client| players.0.get(client).copied())
        .collect();
    if members.is_empty() {
        return vec![credited];
    }
    let counts = |member: Entity| {
        let near = member == credited
            || at
                .zip(position(world, member))
                .is_some_and(|(at, stands)| at.distance(stands) <= rules.experience.range);
        near && !stat::is_dead(world, member) && world.get::<Away>(member).is_none()
    };
    members
        .into_iter()
        .filter(|&member| counts(member))
        .collect()
}

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
