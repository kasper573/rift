use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Action, Actor, ActorModel};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::math::Direction;
use crate::core::script::{Answer, Compiler, EffectDef, EffectStep, Name, ScriptFault};
use crate::core::time::{PlaybackRate, UnixMillis, WallClock};
use crate::systems::movement;
use crate::systems::notification;
use crate::systems::script::effect::{self as effects, Aim, Arrow, Audience, Link};
use crate::systems::script::{EffectCall, FlowKey, Reach, RiftInstances, RiftScripting};
use crate::systems::stat::{self, StatKind};

/// An action an actor was told to play, with the moments its model lands and ends it. Replicated,
/// so every client animates the same stroke from the same start.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Playing {
    pub action: Action,
    pub started: UnixMillis,
    pub apex: UnixMillis,
    pub ends: UnixMillis,
}

/// When the actor's last played action ended, or began when nothing was played: the moment the
/// attack delay counts from.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct LastAction {
    pub at: UnixMillis,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Awaited {
    flow: FlowKey,
    wait: Wait,
    apex_answered: bool,
}

#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wait {
    #[default]
    No,
    Apex,
    End,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Animate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<SfxId>,
    #[serde(default, skip_serializing_if = "Wait::is_no")]
    pub wait: Wait,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted: Arrow,
}

const NEXT: Name = "next";
const INTERRUPTED: Name = "interrupted";

impl Wait {
    fn is_no(&self) -> bool {
        *self == Wait::No
    }
}

impl EffectDef<RiftScripting> for Animate {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![
            (NEXT, self.next.as_deref()),
            (INTERRUPTED, self.interrupted.as_deref()),
        ]
    }
    fn pauses(&self) -> bool {
        self.wait != Wait::No
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let (action, sfx, wait) = (self.action, self.sfx, self.wait);
        Ok(Box::new(move |world, flow, cx| {
            let Some(actor) = effects::aimed(world, cx, &who) else {
                return EffectStep::Output(INTERRUPTED);
            };
            if play(world, actor, action, None).is_none() {
                return EffectStep::Output(INTERRUPTED);
            }
            if let Some(sfx) = sfx {
                notification::play_sound(world, Some(actor), cx.subject, sfx, Audience::Everyone);
            }
            if wait == Wait::No {
                return EffectStep::Done;
            }
            world.entity_mut(actor).insert(Awaited {
                flow,
                wait,
                apex_answered: false,
            });
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Animates]
    }
}

/// Starts an action on an actor, facing `toward` when given, timed by its model at its attack
/// speed. A new action replaces one still playing, which then never lands.
pub fn play(
    world: &mut World,
    actor: Entity,
    action: Action,
    toward: Option<Direction>,
) -> Option<Playing> {
    let content = world.resource::<Content>().clone();
    let now = world.resource::<WallClock>().now;
    let speed = PlaybackRate(stat::effective(world, actor, StatKind::AttackSpeed)).at_least(0.01);
    let mut component = world.get_mut::<Actor>(actor)?;
    let dir = toward.unwrap_or(component.dir);
    super::set_facing(&mut component, dir, action);
    component.attack_rate = speed;
    let model = component.model;
    let timing = ActorModel::get(&content, model).timing(action, dir);
    let rate = if action == Action::Attack {
        speed
    } else {
        PlaybackRate(1.0)
    };
    let playing = Playing {
        action,
        started: now,
        apex: now.after(timing.apex / rate),
        ends: now.after(timing.duration / rate),
    };
    interrupt(world, actor);
    world.entity_mut(actor).insert(playing);
    Some(playing)
}

pub fn playing(world: &World, actor: Entity) -> bool {
    world.get::<Playing>(actor).is_some()
}

pub fn progress(world: &mut World, playing: &mut QueryState<Entity, With<Playing>>) {
    let now = world.resource::<WallClock>().now;
    let actors: Vec<Entity> = playing.iter(world).collect();
    for actor in actors {
        let Some(played) = world.get::<Playing>(actor).copied() else {
            continue;
        };
        if stat::is_dead(world, actor) || movement::moving(world, actor) {
            interrupt(world, actor);
            world.entity_mut(actor).insert(LastAction { at: now });
            continue;
        }
        if now >= played.apex
            && let Some(mut awaited) = world.get_mut::<Awaited>(actor)
            && awaited.wait == Wait::Apex
            && !awaited.apex_answered
        {
            awaited.apex_answered = true;
            let flow = awaited.flow;
            answer(world, flow, NEXT);
        }
        if now >= played.ends {
            world
                .entity_mut(actor)
                .remove::<Playing>()
                .insert(LastAction { at: played.ends });
            if let Some(awaited) = world.entity_mut(actor).take::<Awaited>()
                && (awaited.wait == Wait::End || !awaited.apex_answered)
            {
                answer(world, awaited.flow, NEXT);
            }
        }
    }
}

/// Whether the actor may begin another action: nothing playing, and the attack delay over.
pub fn ready(world: &World, actor: Entity) -> bool {
    if playing(world, actor) {
        return false;
    }
    let now = world.resource::<WallClock>().now;
    let delay = crate::core::time::Millis(stat::effective(world, actor, StatKind::AttackDelay));
    world
        .get::<LastAction>(actor)
        .is_none_or(|last| now.since(last.at) >= delay.seconds())
}

fn interrupt(world: &mut World, actor: Entity) {
    let mut entity = world.entity_mut(actor);
    let awaited = entity.take::<Awaited>();
    entity.remove::<Playing>();
    if let Some(awaited) = awaited
        && (awaited.wait == Wait::End || !awaited.apex_answered)
    {
        answer(world, awaited.flow, INTERRUPTED);
    }
}

fn answer(world: &mut World, flow: FlowKey, output: Name) {
    world
        .resource_mut::<RiftInstances>()
        .answer(flow, Answer::Output(output));
}
