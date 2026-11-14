use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, FlowCx, NEXT, ScriptFault};
use crate::systems::combat;
use crate::systems::npc::Npc;
use crate::systems::script::effect::{self as effects, AimedAll, Reaching};
use crate::systems::script::{Condition, EffectCall, EffectReach, RiftScripting};
use crate::systems::stat::{self, Vital};

/// Restores health, or mana, up to its maximum.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("stat"))]
pub struct Heal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vital::is_health")]
    pub restores: Vital,
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

/// Takes health, credited to an npc owner, or mana, which is a blow to nobody.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("stat"))]
pub struct Damage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vital::is_health")]
    pub takes: Vital,
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
        let restores = self.restores;
        Ok(Box::new(move |world, flow, cx| {
            let amount = |world: &World| by.as_ref().map(|by| by(world, cx).as_num() as f32);
            match effects::aimed_all_anywhere(world, cx, &who) {
                AimedAll::Here(actors) => {
                    for actor in actors {
                        let amount = amount(world);
                        mend(world, actor, restores, amount);
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
                            mend(world, actor, restores, amount);
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
        let takes = self.takes;
        Ok(Box::new(move |world, flow, cx| {
            match effects::aimed_all_anywhere(world, cx, &who) {
                AimedAll::Here(actors) => {
                    for actor in actors {
                        let amount = by(world, cx).as_num() as f32;
                        let attacker = attacker(world, cx, actor);
                        take(world, actor, takes, attacker, amount);
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
                            take(world, actor, takes, None, amount);
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

/// Gives `actor` `amount` of a vital back, or all of it when none is given. Health restored
/// this way is a heal, shown where it lands.
fn mend(world: &mut World, actor: Entity, restores: Vital, amount: Option<f32>) {
    match (restores, amount) {
        (Vital::Health, Some(amount)) => stat::heal(world, actor, amount),
        (Vital::Mana, Some(amount)) => stat::restore(world, actor, Vital::Mana, amount),
        (_, None) => stat::refill(world, actor, restores),
    }
}

/// Takes `amount` of a vital from `actor`: health as a blow of `attacker`'s, mana quietly.
fn take(world: &mut World, actor: Entity, takes: Vital, attacker: Option<Entity>, amount: f32) {
    match takes {
        Vital::Health => combat::inflict(world, actor, attacker, amount),
        Vital::Mana => stat::drain(world, actor, Vital::Mana, amount),
    }
}

/// Who the hurt actor holds responsible: an npc's own script, or the caster whose skill ran.
fn attacker(world: &World, cx: &FlowCx, target: Entity) -> Option<Entity> {
    cx.owner
        .filter(|owner| world.get::<Npc>(*owner).is_some())
        .or_else(|| cx.subject.filter(|subject| *subject != target))
}
