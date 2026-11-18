//! What scripts do with instances: let someone into theirs, send someone out of the one they
//! are in, and count a task of it as done.

use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::systems::item::effects::PaysItems;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AreaInstanceEntry, AreaInstanceGoal, GoalTask};
use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, REFUSED, ScriptFault};
use crate::core::script::{EffectCall, Expr, Fill};
use crate::systems::aim::{self as effects, Reaching};
use crate::systems::area::AreaDef;
use crate::systems::area::Place;
use crate::systems::reward::Reward;
use crate::systems::worlds::WorldName;

/// Lets a player into an instance of an instanced area: with these rewards for clearing it, and this
/// place to come back to. Every start goes through the entrance panel, where the player inserts
/// what the area takes and chooses to go alone, or, leading a party, to gather it. Rewards with
/// items are for a game that has messages, where what a bag has no room for is sent.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("instance"))]
pub struct EnterInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    /// The instanced area, and where in it they appear.
    pub to: Fill<Place>,
    pub rewards: Reward,
    /// Where leaving the instance leads.
    pub return_to: Fill<Place>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when they are not let in: they leave the entrance panel, the call they made ends
    /// without them, what the area takes is gone from their bag, or the server runs as many as
    /// it is set to. Left unwired, they are told why.
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub refused: Arrow,
}

impl EffectDef for EnterInstance {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, REFUSED => refused);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
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
        let camera = cx.camera();
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
            super::entrance::enter(world, character, cx.owner, entry, camera);
            EffectStep::Pause
        }))
    }
    fn reach(&self, cx: &Compiler) -> Vec<Reach> {
        let enters = cx
            .peek_place(&self.to)
            .zip(cx.peek_place(&self.return_to))
            .map(|(to, return_to)| Reach::new(EntersInstance { to, return_to }));
        let pays = (!self.rewards.items.is_empty()).then(|| Reach::new(PaysItems));
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
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for LeaveInstance {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
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
}

/// Counts a task among the goals of the instance the script runs in as done. Nothing happens
/// outside an instance, to a task done already, or to one the area does not ask for.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("instance"))]
pub struct CompleteGoal {
    pub task: GoalTask,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for CompleteGoal {
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let task = self.task.clone();
        let goal_of_some_area = cx
            .content
            .table::<AreaDef>()
            .rows()
            .filter_map(|area| area.instance.as_ref())
            .flat_map(|rules| &rules.goals)
            .any(
                |goal| matches!(goal, AreaInstanceGoal::Task { task: known, .. } if *known == task),
            );
        if !goal_of_some_area {
            return Err(ScriptFault::new(
                "unknown task",
                format!("no instanced area has a goal task {}", task.0),
            ));
        }
        Ok(Box::new(move |world, _, _| {
            super::complete(world, task.clone());
            EffectStep::Done
        }))
    }
}

/// A script that may send a player into an instance of the area `to`, and out again to `return_to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntersInstance {
    pub to: Place,
    pub return_to: Place,
}

impl ReachDef for EntersInstance {}
