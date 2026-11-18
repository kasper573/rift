use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Home;
use crate::core::content::Content;
use crate::core::content::SchemaMark;
use crate::core::math::{Offset, Pos};
use crate::core::script::{
    Arrow, Compiler, DONE, EffectDef, EffectStep, NEXT, REFUSED, ScriptFault,
};
use crate::core::script::{EffectCall, EffectLook, Expr, Reach, ReachDef, RunnerKind, Vocabulary};
use crate::core::text::LineText;
use crate::core::units::Meters;
use crate::systems::aim as effects;
use crate::systems::area::spawnable::SpawnedObject;
use crate::systems::area::{self, AreaTag, object};
use crate::systems::combat::Attitude;
use crate::systems::messages;
use crate::systems::movement::{self, position};
use crate::systems::npc::NPC_RUNNER;
use crate::systems::npc::NpcId;
use crate::systems::player::{ClientId, PlayedBy};
use crate::systems::stat;
use crate::systems::visibility::Presence;

/// Hostile or friendly, until set again.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("npc"))]
pub struct SetAttitude {
    pub attitude: Attitude,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Spawns a pack of npcs nearby.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("npc"))]
pub struct SpawnNpcs {
    pub npc: NpcId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub count: u32,
    /// How far from whom they appear beside the npcs stand, in a ring around them.
    #[schemars(extend("default" = 1.5))]
    pub spread: Meters,
    pub near: SpawnNear,
    pub shown: ShownFor,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnNear {
    Owner,
    Subject,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShownFor {
    Subject,
    Everyone,
}

/// Removes an npc that spawn_npcs made, or an object that spawn_object made.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("npc"))]
pub struct Despawn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub done: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summoned {
    pub with: Option<Entity>,
}

impl EffectDef for SetAttitude {
    fn owners(&self) -> &'static [RunnerKind] {
        &[NPC_RUNNER]
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile(&self, _cx: &mut Compiler) -> Result<EffectCall, ScriptFault> {
        let attitude = self.attitude;
        Ok(Box::new(move |world, _, cx| {
            if let Some(npc) = cx.owner
                && world.get_entity(npc).is_ok()
            {
                world.entity_mut(npc).insert(attitude);
            }
            EffectStep::Done
        }))
    }
    fn look(&self, _cx: &mut Compiler) -> Result<Option<EffectLook>, ScriptFault> {
        let attitude = self.attitude;
        Ok(Some(Box::new(move |world, _, cx| {
            let npc = cx.owner?;
            (world.get::<Attitude>(npc) == Some(&attitude)).then_some(EffectStep::Done)
        })))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![match self.attitude {
            Attitude::Hostile => Reach::new(TurnsHostile),
            Attitude::Friendly => Reach::new(TurnsFriendly),
        }]
    }
}

impl EffectDef for SpawnNpcs {
    fn needs_subject(&self) -> bool {
        self.near == SpawnNear::Subject || self.shown == ShownFor::Subject
    }
    crate::outputs!(NEXT => next);
    fn compile(&self, _cx: &mut Compiler) -> Result<EffectCall, ScriptFault> {
        if self.count == 0 {
            return Err(ScriptFault::new(
                "out of range",
                "a count of at least 1, not 0",
            ));
        }
        let (npc, count, spread, near, shown) =
            (self.npc, self.count, self.spread, self.near, self.shown);
        Ok(Box::new(move |world, _, cx| {
            let beside = match near {
                SpawnNear::Owner => cx.owner.or(cx.subject),
                SpawnNear::Subject => cx.subject,
            };
            let Some(beside) = beside else {
                return EffectStep::Done;
            };
            let only_for = match shown {
                ShownFor::Subject => cx
                    .subject
                    .and_then(|subject| world.get::<PlayedBy>(subject))
                    .map(|owner| owner.client),
                ShownFor::Everyone => None,
            };
            summon(
                world,
                npc,
                count,
                spread,
                beside,
                only_for,
                cx.subject.filter(|_| shown == ShownFor::Subject),
            );
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Spawns(self.npc))]
    }
}

impl EffectDef for Despawn {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(DONE => done, REFUSED => refused);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let refused = self.refused.clone();
        Ok(Box::new(move |world, _, cx| {
            let Some(spawned) = effects::aimed(world, cx, &who) else {
                let reason = LineText::from(
                    messages::of(world.resource::<Content>())
                        .dialogue
                        .nobody_to_send_away(),
                );
                return effects::refuse(world, cx, &refused, reason);
            };
            if world.get::<Summoned>(spawned).is_some() {
                super::despawn(world, spawned);
            } else if world.get::<SpawnedObject>(spawned).is_some() {
                object::remove(world, spawned);
            } else {
                let reason = LineText::from(
                    messages::of(world.resource::<Content>())
                        .dialogue
                        .only_spawned(),
                );
                return effects::refuse(world, cx, &refused, reason);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Despawns)]
    }
}

pub fn summon(
    world: &mut World,
    npc: NpcId,
    count: u32,
    spread: Meters,
    beside: Entity,
    only_for: Option<ClientId>,
    with: Option<Entity>,
) -> Vec<Entity> {
    let content = world.resource::<Content>().clone();
    let (Some(at), Some(tag), Some(region)) = (
        position(world, beside),
        world.get::<AreaTag>(beside).cloned(),
        area::of_in(&content, world, beside),
    ) else {
        return Vec::new();
    };
    let pack = super::next_pack(world);
    let mut spawned = Vec::with_capacity(count as usize);
    for index in 0..count {
        let spot = movement::place_beside(
            world,
            tag.area,
            &region.mesh(),
            ring(region, at, spread, index, count),
        );
        let summon = super::spawn(world, npc, spot, tag.area, Some(pack));
        world
            .entity_mut(summon)
            .insert((Summoned { with }, Home(spot)));
        if let Some(client) = only_for {
            world.entity_mut(summon).insert(Presence::For(client));
        }
        spawned.push(summon);
    }
    spawned
}

pub fn dismiss(world: &mut World, summons: &mut QueryState<(Entity, &'static Summoned)>) {
    let dismissed: Vec<Entity> = summons
        .iter(world)
        .filter_map(|(summon, summoned)| Some((summon, summoned.with?)))
        .filter(|&(summon, with)| {
            world.get_entity(with).is_err()
                || stat::is_dead(world, with)
                || !area::same_area(world, with, summon)
        })
        .map(|(summon, _)| summon)
        .collect();
    for summon in dismissed {
        super::despawn(world, summon);
    }
}

fn ring(
    region: &area::Area,
    at: Pos<Meters>,
    spread: Meters,
    index: u32,
    count: u32,
) -> Pos<Meters> {
    let angle = std::f32::consts::TAU * index as f32 / count as f32;
    let spot = at + Offset::new(angle.cos() * spread.0, angle.sin() * spread.0);
    region.mesh().nearest(spot).unwrap_or(at)
}

/// A script that may turn an npc hostile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnsHostile;

impl ReachDef for TurnsHostile {}

/// A script that may turn an npc friendly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnsFriendly;

impl ReachDef for TurnsFriendly {}

/// A script that may spawn an npc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawns(pub NpcId);

impl ReachDef for Spawns {}

/// A script that may take an npc out of the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Despawns;

impl ReachDef for Despawns {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<SetAttitude>("set_attitude")
        .effect::<SpawnNpcs>("spawn_npcs")
        .effect::<Despawn>("despawn");
}
