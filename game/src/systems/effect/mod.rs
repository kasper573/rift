pub mod widget;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::time::Seconds;
use crate::systems::stat::{self, Stat, StatKind};

const CHASE_SPEED_MULTIPLIER: f32 = 2.0;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<TimedEffects>().init_resource::<Sources>();
    source(app, timed);
}

#[derive(Clone, Copy)]
pub struct EffectContext<'a> {
    pub world: &'a World,
    pub source: Entity,
    pub target: Entity,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    StatModifier(Stat),
    Chasing,
}

impl Effect {
    pub fn icon(self) -> Option<&'static str> {
        match self {
            Effect::StatModifier(_) | Effect::Chasing => None,
        }
    }

    pub fn compute(self, ctx: &EffectContext) -> Vec<Stat> {
        match self {
            Effect::StatModifier(stat) => vec![stat],
            Effect::Chasing => {
                let base = stat::base(ctx.world, ctx.source, StatKind::MovementSpeed);
                vec![StatKind::MovementSpeed.of(base * (CHASE_SPEED_MULTIPLIER - 1.0))]
            }
        }
    }

    pub fn describe(self, ctx: &EffectContext) -> String {
        self.compute(ctx)
            .iter()
            .map(|stat| format!("{:+} {}", stat.value, stat.label()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TimedEffects(pub Vec<TimedEffect>);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct TimedEffect {
    pub effect: Effect,
    pub until: Seconds,
}

pub type Source = fn(&World, Entity) -> Vec<Effect>;

#[derive(Resource, Default)]
pub struct Sources(Vec<Source>);

pub fn source(app: &mut App, source: Source) {
    app.world_mut().resource_mut::<Sources>().0.push(source);
}

pub fn active_effects(world: &World, entity: Entity) -> Vec<Effect> {
    world
        .resource::<Sources>()
        .0
        .iter()
        .flat_map(|source| source(world, entity))
        .collect()
}

pub fn expire(world: &mut World, timed: &mut QueryState<Entity, With<TimedEffects>>) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let ids: Vec<Entity> = timed.iter(world).collect();
    for id in ids {
        if let Some(mut timed) = world.get_mut::<TimedEffects>(id)
            && timed.0.iter().any(|effect| effect.until <= now)
        {
            timed.0.retain(|effect| effect.until > now);
        }
    }
}

fn timed(world: &World, entity: Entity) -> Vec<Effect> {
    world
        .get::<TimedEffects>(entity)
        .map(|timed| timed.0.iter().map(|timed| timed.effect).collect())
        .unwrap_or_default()
}
