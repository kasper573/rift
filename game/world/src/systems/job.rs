use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::data;
use crate::systems::effect::{self, Effect};
use crate::systems::player::Xp;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Job>();
    effect::source(app, level_effects);
}

fn level_effects(world: &World, entity: Entity) -> Vec<Effect> {
    let Some(job) = world.get::<Job>(entity) else {
        return Vec::new();
    };
    let level = level(world, entity) as usize;
    job.def
        .get()
        .levels
        .iter()
        .take(level)
        .flat_map(|tier| tier.effects.iter().copied())
        .collect()
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Job {
    pub def: data::job::Id,
}

pub struct JobDef {
    pub name: &'static str,
    pub levels: &'static [JobLevel],
}

pub struct JobLevel {
    pub exp: u32,
    pub effects: &'static [Effect],
}

pub fn default_job() -> data::job::Id {
    data::job::Id::Adventurer
}

pub fn level(world: &World, entity: Entity) -> u32 {
    let Some(job) = world.get::<Job>(entity) else {
        return 0;
    };
    let xp = world.get::<Xp>(entity).map_or(0, |xp| xp.amount);
    job.def
        .get()
        .levels
        .iter()
        .filter(|tier| tier.exp <= xp)
        .count() as u32
}
