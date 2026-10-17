use std::collections::HashMap;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Cooldowns, Learned};
use crate::core::script::{Answer, Compiler, EffectDef, EffectStep, Name, ScriptFault};
use crate::systems::class::{self, SkillPoints};
use crate::systems::history::{HistoryTopic, RecordTally};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::{self, sender_player};
use crate::systems::script::effect::{self as effects, Aim, Arrow, Link};
use crate::systems::script::{EffectCall, FlowKey, Reach, RiftInstances, RiftScripting};
use crate::systems::text::LineText;
use crate::systems::{drain_requests, stat};

/// Asks the player before forgetting every learned rank and returning the points. The answer
/// picks the output, so whatever the reset costs is wired after it by the script.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ResetSkills {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

const ACCEPTED: Name = "accepted";
const DECLINED: Name = "declined";

impl EffectDef<RiftScripting> for ResetSkills {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![
            (ACCEPTED, self.accepted.as_deref()),
            (DECLINED, self.declined.as_deref()),
        ]
    }
    fn pauses(&self) -> bool {
        true
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
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
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::ResetsSkills]
    }
}

pub fn answers(world: &mut World) {
    for request in drain_requests::<ResetAnswerRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let Some(flow) = world.resource_mut::<PendingResets>().0.remove(&player) else {
            continue;
        };
        world.entity_mut(player).remove::<ResetPrompt>();
        if request.message.accepted {
            reset(world, player);
            answer(world, flow, ACCEPTED);
        } else {
            answer(world, flow, DECLINED);
        }
    }
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
        if gone || stat::is_dead(world, player) || player::held(world, player) {
            world.resource_mut::<PendingResets>().0.remove(&player);
            if !gone {
                world.entity_mut(player).remove::<ResetPrompt>();
            }
            answer(world, flow, DECLINED);
        }
    }
}

/// Forgets every learned rank and cooldown; the points come back as the class level grants them.
pub fn reset(world: &mut World, player: Entity) {
    let returned = class::spent_points(world, player);
    if let Some(mut learned) = world.get_mut::<Learned>(player) {
        learned.0.clear();
    }
    if let Some(mut cooldowns) = world.get_mut::<Cooldowns>(player) {
        cooldowns.0.clear();
    }
    notification::notify(
        world,
        player,
        Notification::new(
            NotificationKind::Feed {
                topic: HistoryTopic::Notification,
                icon: None,
                tally: Some(RecordTally::Change(returned.0 as i32)),
                failure: false,
                sfx: None,
            },
            LineText::plain("Skills reset, skill points returned"),
        ),
    );
}

pub fn prompt(world: &World, player: Entity) -> ResetPrompt {
    let learned = world.get::<Learned>(player);
    ResetPrompt {
        ranks: learned.map_or(0, |learned| learned.ranks() as u32),
        skills: learned.map_or(0, |learned| {
            learned.0.values().filter(|rank| rank.0 > 0).count() as u32
        }),
        points: class::spent_points(world, player),
    }
}

fn answer(world: &mut World, flow: FlowKey, output: Name) {
    world
        .resource_mut::<RiftInstances>()
        .answer(flow, Answer::Output(output));
}

impl ResetPrompt {
    pub fn describe(self) -> String {
        let prompt = self;
        format!(
            "Forget {} rank{} across {} skill{}; {} point{} come back",
            prompt.ranks,
            if prompt.ranks == 1 { "" } else { "s" },
            prompt.skills,
            if prompt.skills == 1 { "" } else { "s" },
            prompt.points.0,
            if prompt.points.0 == 1 { "" } else { "s" },
        )
    }
}
