use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{Compiler, EffectDef, EffectStep, FlowCx, Name, ScriptFault};
use crate::systems::combat;
use crate::systems::npc::Npc;
use crate::systems::script::effect::{self as effects, Aim, Arrow, Link};
use crate::systems::script::{Condition, EffectCall, Reach, RiftScripting};

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Heal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub by: HealBy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HealBy {
    Full(Fully),
    Amount(Condition),
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fully {
    Full,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Damage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub by: Condition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for Heal {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![("next", self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all(cx, &self.who)?;
        let by = match &self.by {
            HealBy::Amount(amount) => Some(effects::number(cx, amount)?),
            HealBy::Full(_) => None,
        };
        Ok(Box::new(move |world, _, cx| {
            for actor in effects::aimed_all(world, cx, &who) {
                match &by {
                    Some(by) => {
                        let amount = by(world, cx).as_num() as f32;
                        super::heal(world, actor, amount);
                    }
                    None => super::refill(world, actor),
                }
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Heals]
    }
}

impl EffectDef<RiftScripting> for Damage {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![("next", self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_all(cx, &self.who)?;
        let by = effects::number(cx, &self.by)?;
        Ok(Box::new(move |world, _, cx| {
            for actor in effects::aimed_all(world, cx, &who) {
                let amount = by(world, cx).as_num() as f32;
                let attacker = attacker(world, cx, actor);
                combat::inflict(world, actor, attacker, amount);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Damages]
    }
}

/// Who the hurt actor holds responsible: an npc's own script, or the caster whose skill ran.
fn attacker(world: &World, cx: &FlowCx, target: Entity) -> Option<Entity> {
    cx.owner
        .filter(|owner| world.get::<Npc>(*owner).is_some())
        .or_else(|| cx.subject.filter(|subject| *subject != target))
}
