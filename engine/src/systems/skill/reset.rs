use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::Vocabulary;
use std::collections::HashMap;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Learned, SkillMessages};
use crate::core::content::{Content, SchemaMark};
use crate::core::script::{
    ACCEPTED, Answer, Arrow, Compiler, DECLINED, EffectDef, EffectStep, ScriptFault,
};
use crate::core::script::{EffectCall, Expr, FlowKey, Instances};
use crate::systems::aim as effects;
use crate::systems::class::{self, SkillPoints};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::messages;
use crate::systems::notification::{self, Notification};
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, player_requests};
use crate::systems::stat;

/// Asks the player before forgetting every learned rank and returning the points. The answer
/// picks the output, so whatever the reset costs is wired after it by the script.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("skill"))]
pub struct ResetSkills {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub accepted: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub declined: Arrow,
}

/// The question put to the player, replicated to them alone while it waits.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ResetPrompt {
    pub ranks: u32,
    pub skills: u32,
    pub points: SkillPoints,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ResetAnswerRequest {
    pub accepted: bool,
}

#[derive(Resource, Default)]
pub struct PendingResets(HashMap<Entity, FlowKey>);

impl EffectDef for ResetSkills {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(ACCEPTED => accepted, DECLINED => declined);
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        Ok(Box::new(move |world, flow, cx| {
            let Some(player) = effects::aimed_character(world, cx, &who) else {
                return EffectStep::Output(DECLINED);
            };
            if let Some(earlier) = world.resource_mut::<PendingResets>().0.insert(player, flow) {
                answer(world, earlier, DECLINED);
            }
            let asked = prompt(world, player);
            world.entity_mut(player).insert(asked);
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(ResetsSkills)]
    }
}

pub fn answers(world: &mut World) {
    player_requests::<ResetAnswerRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            let Some(flow) = world.resource_mut::<PendingResets>().0.remove(&player) else {
                return Ok(());
            };
            world.entity_mut(player).remove::<ResetPrompt>();
            if request.accepted {
                reset(world, player);
                answer(world, flow, ACCEPTED);
            } else {
                answer(world, flow, DECLINED);
            }
            Ok(())
        },
    );
}

/// A prompt nobody can answer any more, because its player died, left or is being moved, is
/// declined for them.
pub fn expire(world: &mut World) {
    let pending: Vec<(Entity, FlowKey)> = world
        .resource::<PendingResets>()
        .0
        .iter()
        .map(|(player, flow)| (*player, *flow))
        .collect();
    for (player, flow) in pending {
        let gone = world.get_entity(player).is_err();
        if gone || stat::is_dead(world, player) || player::arriving(world, player) {
            world.resource_mut::<PendingResets>().0.remove(&player);
            if !gone {
                world.entity_mut(player).remove::<ResetPrompt>();
            }
            answer(world, flow, DECLINED);
        }
    }
}

/// Forgets every learned rank; the points come back as the class level grants them. A cooldown
/// still running runs on: a reset is no way to be rid of one.
pub fn reset(world: &mut World, player: Entity) {
    let returned = class::spent_points(world, player);
    if let Some(mut learned) = world.get_mut::<Learned>(player) {
        learned.0.clear();
    }
    let content = world.resource::<Content>().clone();
    let said = messages::of(&content).skill.skills_reset().to_owned();
    notification::notify(
        world,
        player,
        None,
        Notification::feed(HistoryTopic::Notification, said).tally(RecordTally::Change(
            i32::try_from(returned.0).unwrap_or(i32::MAX),
        )),
    );
}

pub fn prompt(world: &World, player: Entity) -> ResetPrompt {
    let learned = world.get::<Learned>(player);
    ResetPrompt {
        ranks: learned.map_or(0, Learned::ranks),
        skills: learned.map_or(0, |learned| learned.skills_held()),
        points: class::spent_points(world, player),
    }
}

impl ResetPrompt {
    pub fn describe(self, words: &SkillMessages) -> String {
        words.reset_summary(self.points.0, self.ranks, self.skills)
    }
}

fn answer(world: &mut World, flow: FlowKey, output: &'static str) {
    world
        .resource_mut::<Instances>()
        .answer(flow, Answer::Output(output));
}

/// A script that may reset a player's skills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResetsSkills;

impl ReachDef for ResetsSkills {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.effect::<ResetSkills>("reset_skills");
}
