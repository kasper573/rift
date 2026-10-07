use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::data;
use crate::systems::effect::{self, Effect};
use crate::systems::player::Xp;
use crate::systems::rule::Requirement;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Job>();
    effect::source(app, level_effects);
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

pub struct MinLevel(pub u32);

impl Requirement for MinLevel {
    fn met(&self, world: &World, player: Entity) -> bool {
        level(world, player) >= self.0
    }

    fn describe(&self) -> String {
        format!("Level {}", self.0)
    }
}

pub struct IsJob(pub data::job::Id);

impl Requirement for IsJob {
    fn met(&self, world: &World, player: Entity) -> bool {
        world
            .get::<Job>(player)
            .is_some_and(|job| job.def == self.0)
    }

    fn describe(&self) -> String {
        self.0.get().name.to_owned()
    }
}
