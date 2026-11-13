use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, FlowCx, NEXT, ScriptFault};
use crate::systems::npc::Npc;
use crate::systems::script::effect::{self as effects, AimedAll, Reaching};
use crate::systems::script::{Condition, EffectCall, EffectReach, RiftScripting};
use crate::systems::{combat, stat};

/// Restores health up to the maximum.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("stat"))]
pub struct Heal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub by: HealBy,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HealBy {
    Full(Fully),
    #[schemars(extend("default" = 10.0))]
    Amount(Condition),
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fully {
    Full,
}

/// Takes health, credited to an npc owner.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("stat"))]
pub struct Damage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[schemars(extend("default" = 5.0))]
    pub by: Condition,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Heal {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all_anywhere(cx, &self.who)?;
        let by = match &self.by {
            HealBy::Amount(amount) => Some(effects::number(cx, amount)?),
            HealBy::Full(_) => None,
        };
        Ok(Box::new(move |world, flow, cx| {
            let amount = |world: &World| by.as_ref().map(|by| by(world, cx).as_num() as f32);
            match effects::aimed_all_anywhere(world, cx, &who) {
                AimedAll::Here(actors) => {
                    for actor in actors {
                        let amount = amount(world);
                        mend(world, actor, amount);
                    }
                    EffectStep::Done
                }
                AimedAll::Elsewhere(client) => {
                    let amount = amount(world);
                    effects::carry(
                        world,
                        flow,
                        cx,
                        client,
                        Reaching::quietly(NEXT),
                        move |world, actor| {
                            mend(world, actor, amount);
                            Ok(())
                        },
                    )
                }
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Heals]
    }
}

impl EffectDef<RiftScripting> for Damage {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all_anywhere(cx, &self.who)?;
        let by = effects::number(cx, &self.by)?;
        Ok(Box::new(move |world, flow, cx| {
            match effects::aimed_all_anywhere(world, cx, &who) {
                AimedAll::Here(actors) => {
                    for actor in actors {
                        let amount = by(world, cx).as_num() as f32;
                        let attacker = attacker(world, cx, actor);
                        combat::inflict(world, actor, attacker, amount);
                    }
                    EffectStep::Done
                }
                // Whoever is to blame is an entity of this world: hurt carried to another has
                // no attacker there.
                AimedAll::Elsewhere(client) => {
                    let amount = by(world, cx).as_num() as f32;
                    effects::carry(
                        world,
                        flow,
                        cx,
                        client,
                        Reaching::quietly(NEXT),
                        move |world, actor| {
                            combat::inflict(world, actor, None, amount);
                            Ok(())
                        },
                    )
                }
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        vec![EffectReach::Damages]
    }
}

/// Heals `actor` by `amount`, or in full when none is given.
fn mend(world: &mut World, actor: Entity, amount: Option<f32>) {
    match amount {
        Some(amount) => stat::heal(world, actor, amount),
        None => stat::refill(world, actor),
    }
}

/// Who the hurt actor holds responsible: an npc's own script, or the caster whose skill ran.
fn attacker(world: &World, cx: &FlowCx, target: Entity) -> Option<Entity> {
    cx.owner
        .filter(|owner| world.get::<Npc>(*owner).is_some())
        .or_else(|| cx.subject.filter(|subject| *subject != target))
}
