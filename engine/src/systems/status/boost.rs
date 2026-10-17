use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::AspectSource;
use crate::systems::stat::Stat;

/// The stat modifiers an actor's active aspects hold on it: summed into its effective stats, let
/// go with the aspect that holds them.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Boosts(pub Vec<HeldBoost>);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct HeldBoost {
    pub source: AspectSource,
    pub stat: Stat,
}

pub fn hold(world: &mut World, actor: Entity, source: AspectSource, stat: Stat) {
    if world.get_entity(actor).is_err() {
        return;
    }
    let held = HeldBoost { source, stat };
    match world.get_mut::<Boosts>(actor) {
        Some(mut boosts) => {
            match boosts
                .0
                .iter_mut()
                .find(|boost| boost.source == source && boost.stat.kind == stat.kind)
            {
                Some(boost) => *boost = held,
                None => boosts.0.push(held),
            }
        }
        None => {
            world.entity_mut(actor).insert(Boosts(vec![held]));
        }
    }
}

pub fn drop(world: &mut World, actor: Entity, source: AspectSource) {
    if let Some(mut boosts) = world.get_mut::<Boosts>(actor)
        && boosts.0.iter().any(|boost| boost.source == source)
    {
        boosts.0.retain(|boost| boost.source != source);
    }
}

pub fn modifiers(world: &World, entity: Entity) -> Vec<Stat> {
    world
        .get::<Boosts>(entity)
        .map(|boosts| boosts.0.iter().map(|boost| boost.stat).collect())
        .unwrap_or_default()
}
