//! What scripts do with instances: let someone into theirs, send someone out of the one they
//! are in, and count a task of it as done.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AreaInstanceEntry, AreaInstanceRewards};
use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, REFUSED, ScriptFault};
use crate::systems::script::effect::{self as effects, Reaching};
use crate::systems::script::{Condition, EffectCall, EffectReach, Fill, Place, RiftScripting};
use crate::systems::worlds::WorldName;

/// Lets a player into a copy of an instanced area: with these rewards for clearing it, and this
/// place to come back to. A player in no party, at an area that may be done alone, goes in at
/// once; anyone else chooses at the entrance panel to go alone, or, leading a party, to gather
/// it. Rewards with items are for a game that has messages, where what a bag has no room for is
/// sent.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("instance"))]
pub struct EnterInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    /// The instanced area, and where in it they appear.
    pub to: Fill<Place>,
    pub rewards: AreaInstanceRewards,
    /// Where leaving the instance leads.
    pub return_to: Fill<Place>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when they are not let in: they leave the entrance panel, the call they made ends
    /// without them, or the server runs as many as it is set to. Left unwired, they are told why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef<RiftScripting> for EnterInstance {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn pauses(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let to = cx.place(&self.to)?;
        let return_to = cx.place(&self.return_to)?;
        if to.area.get(cx.content).instance.is_none() {
            return Err(ScriptFault::new(
                "not an instanced area",
                "an area whose config has an instance block to enter",
            ));
        }
        if return_to.area.get(cx.content).instance.is_some() {
            return Err(ScriptFault::new(
                "returns into an instance",
                "a return point in an area everyone shares",
            ));
        }
        let rewards = self.rewards.clone();
        let told = self.refused.is_none();
        Ok(Box::new(move |world, flow, cx| {
            let Some(character) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Output(REFUSED);
            };
            let Some(asked_in) = WorldName::of(world) else {
                return EffectStep::Output(REFUSED);
            };
            let entry = AreaInstanceEntry {
                to,
                rewards: rewards.clone(),
                return_to,
                asked_in,
                flow,
                told,
            };
            super::entrance::enter(world, character, cx.owner, entry);
            EffectStep::Pause
        }))
    }
    fn reach(&self, cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        let enters = cx
            .peek_place(&self.to)
            .zip(cx.peek_place(&self.return_to))
            .map(|(to, return_to)| EffectReach::EntersInstance { to, return_to });
        let pays = (!self.rewards.items.is_empty()).then_some(EffectReach::PaysItems);
        enters.into_iter().chain(pays).collect()
    }
}

/// Sends a player out of the instance they are in, to where leaving it leads, for good. Nothing
/// happens to one who is in none.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("instance"))]
pub struct LeaveInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for LeaveInstance {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim_anywhere(cx, &self.who)?;
        Ok(Box::new(move |world, flow, cx| {
            let aimed = effects::aimed_character_anywhere(world, cx, &who);
            effects::reach(
                world,
                flow,
                cx,
                aimed,
                Reaching::quietly(NEXT),
                |world, character| {
                    super::leave(world, character);
                    Ok(())
                },
            )
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        Vec::new()
    }
}

/// Counts a task among the goals of the instance the script runs in as done. Nothing happens
/// outside an instance, to a task done already, or to one the area does not ask for.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("instance"))]
pub struct CompleteGoal {
    pub task: String,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for CompleteGoal {
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        _cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let task = self.task.clone();
        Ok(Box::new(move |world, _, _| {
            super::complete(world, &task);
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<EffectReach> {
        Vec::new()
    }
}
