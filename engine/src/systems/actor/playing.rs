use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::held::{self, ScriptedStance};
use super::skeleton::{Declared, GestureName, MoveName};
use super::stance::StanceId;
use super::{Action, Actor, ActorModel, Appearance, Pose};
use crate::core::audio::playback::SfxId;
use crate::core::content::{Content, serial};
use crate::core::geometry::Radians;
use crate::core::math::Rng;
use crate::core::script::{Answer, Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault};
use crate::core::time::{Millis, PlaybackRate, UnixMillis, WallClock};
use crate::systems::script::effect::{self as effects, Audience};
use crate::systems::script::event::NPC;
use crate::systems::script::{
    Condition, EffectCall, FlowKey, Reach, RiftInstances, RiftScripting, RunnerKind,
};
use crate::systems::stat::{self, StatKind};
use crate::systems::{movement, notification};

/// What an actor was told to perform: an engine action, or a move by name, which its stance turns
/// into a clip on its body.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Action(Action),
    Move(MoveName),
    /// Taking a pose: sitting down, lying down.
    Begin(Pose),
    /// Leaving a pose for idling: standing up.
    End(Pose),
    /// A gesture by name, such as a wave: made only by a body whose sets name it.
    Gesture(GestureName),
}

impl Motion {
    /// The action the actor shows while the motion plays: a move is an attack, taking or leaving
    /// a pose or gesturing is idling.
    pub fn action(&self) -> Action {
        match self {
            Motion::Action(action) => *action,
            Motion::Move(_) => Action::Attack,
            Motion::Begin(_) | Motion::End(_) | Motion::Gesture(_) => Action::Idle,
        }
    }

    /// Whether the motion shows while the actor does `action`: a gesture is made standing, idle or
    /// talking, and any other motion while the actor does what it shows.
    pub fn shows_over(&self, action: Action) -> bool {
        match self {
            Motion::Gesture(_) => matches!(action, Action::Idle | Action::Talk),
            _ => self.action() == action,
        }
    }

    /// Whether the motion is a hit, which plays at the actor's attack speed and lands at its apex.
    pub fn strikes(&self) -> bool {
        self.action() == Action::Attack
    }

    /// Whether the motion's clip plays through once and holds its last moment, rather than looping:
    /// a hit lands once, a body falls once, sits down once, waves once.
    pub fn once(&self) -> bool {
        matches!(self, Motion::Begin(_) | Motion::End(_) | Motion::Gesture(_))
            || matches!(self.action(), Action::Attack | Action::Dead | Action::Hurt)
    }
}

/// A motion an actor was told to play, which of its cell's clips plays, and the moments its
/// clip lands and ends it. Replicated, so every client animates the same stroke from the same start.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Playing {
    pub motion: Motion,
    pub pick: usize,
    pub started: UnixMillis,
    pub apex: UnixMillis,
    pub ends: UnixMillis,
}

/// The motion an actor last played and which of its cell's clips played, which the next play
/// of the same motion takes its turn after.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct LastPlayed {
    pub motion: Motion,
    pub pick: usize,
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

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wait {
    Apex,
    End,
}

/// Plays an action, or a move the actor's stance turns into a clip, waiting for its apex or end
/// when asked.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "actor"))]
pub struct Animate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    #[serde(flatten)]
    pub motion: Motion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<SfxId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<Wait>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    /// Taken when the action can't play or is cut short.
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub interrupted: Arrow,
}

const INTERRUPTED: &str = "interrupted";

impl EffectDef<RiftScripting> for Animate {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next, INTERRUPTED => interrupted);
    fn pauses(&self) -> bool {
        self.wait.is_some()
    }
    fn keeps_box(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let declared = cx.content.built::<Declared>();
        let undeclared = match &self.motion {
            Motion::Move(name) if !declared.moves.contains(name) => {
                Some(("undeclared move", "move", name.as_str()))
            }
            Motion::Gesture(name) if !declared.gestures.contains(name) => {
                Some(("undeclared gesture", "gesture", name.as_str()))
            }
            _ => None,
        };
        if let Some((rule, kind, name)) = undeclared {
            return Err(ScriptFault::new(
                rule,
                format!(
                    "a {kind} some skeleton's set declares; animate asks for {kind} {name}, which no skeleton's set declares"
                ),
            ));
        }
        let (motion, sfx, wait) = (self.motion.clone(), self.sfx, self.wait);
        Ok(Box::new(move |world, flow, cx| {
            let Some(actor) = effects::aimed(world, cx, &who) else {
                return EffectStep::Output(INTERRUPTED);
            };
            if !plays(world, actor, &motion) {
                return EffectStep::Done;
            }
            if play(world, actor, motion.clone(), None).is_none() {
                return EffectStep::Output(INTERRUPTED);
            }
            if let Some(sfx) = sfx {
                notification::play_sound(world, Some(actor), cx.subject, sfx, Audience::Everyone);
            }
            let Some(wait) = wait else {
                return EffectStep::Done;
            };
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

/// Starts a motion on an actor, facing `toward` when given, timed by the clip its stance resolves
/// it to on its body, at its attack speed. A new motion replaces one still playing, which then never
/// lands.
pub fn play(
    world: &mut World,
    actor: Entity,
    motion: Motion,
    toward: Option<Radians>,
) -> Option<Playing> {
    let content = world.resource::<Content>().clone();
    let now = world.resource::<WallClock>().now;
    let speed = PlaybackRate(stat::effective(world, actor, StatKind::AttackSpeed));
    let stance = held::stance_of(world, actor);
    let model = ActorModel::get(&content, world.get::<Appearance>(actor)?.model);
    let last = world
        .get::<LastPlayed>(actor)
        .filter(|last| last.motion == motion)
        .map(|last| last.pick);
    let pick = model
        .cell(stance, &motion, Pose::Idle)
        .after(last, &mut world.resource_mut::<Rng>());
    let mut component = world.get_mut::<Actor>(actor)?;
    let facing = toward.unwrap_or(component.facing);
    super::set_facing(&mut component, facing, motion.action());
    component.attack_rate = speed;
    let timing = model.timing(stance, &motion, pick, speed);
    let playing = Playing {
        motion,
        pick,
        started: now,
        apex: now.after(timing.apex),
        ends: now.after(timing.duration),
    };
    interrupt(world, actor);
    world.entity_mut(actor).insert((
        playing.clone(),
        LastPlayed {
            motion: playing.motion.clone(),
            pick,
        },
    ));
    Some(playing)
}

/// Puts an actor in a stance with nothing in its hands, or out of the one a script put it in.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "actor"))]
pub struct SetStance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    /// None takes the actor out of the stance a script put it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stance: Option<StanceId>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for SetStance {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        let stance = self.stance;
        Ok(Box::new(move |world, _, cx| {
            for actor in effects::aimed_all(world, cx, &who) {
                let mut entity = world.entity_mut(actor);
                match stance {
                    Some(stance) => {
                        entity.insert(ScriptedStance(stance));
                    }
                    None => {
                        entity.remove::<ScriptedStance>();
                    }
                }
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Animates]
    }
}

/// Puts the npc in a pose, or back to idling, through the clip its sets name for taking the new
/// pose or leaving the old one, waiting for that clip to play through when asked. A pose whose sets
/// name no such clip is taken or left at once.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "actor"))]
pub struct SetPose {
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub wait: bool,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef<RiftScripting> for SetPose {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn pauses(&self) -> bool {
        self.wait
    }
    fn compile<'c>(
        &'c self,
        _cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let (pose, wait) = (self.pose, self.wait);
        Ok(Box::new(move |world, flow, cx| {
            let Some(actor) = cx.owner else {
                return EffectStep::Done;
            };
            match pose_to(world, actor, pose) {
                Some(_) if wait => {
                    world.entity_mut(actor).insert(Awaited {
                        flow,
                        wait: Wait::End,
                        apex_answered: false,
                    });
                    EffectStep::Pause
                }
                _ => EffectStep::Done,
            }
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Animates]
    }
}

/// Puts an actor in `pose`, playing the clip that takes it, or else the one that leaves the pose
/// it was in, when its sets name one.
pub fn pose_to(world: &mut World, actor: Entity, pose: Pose) -> Option<Playing> {
    let content = world.resource::<Content>().clone();
    let stance = held::stance_of(world, actor);
    let model = ActorModel::get(&content, world.get::<Appearance>(actor)?.model);
    let mut component = world.get_mut::<Actor>(actor)?;
    let from = std::mem::replace(&mut component.pose, pose);
    if from == pose {
        return None;
    }
    let skeleton = &model.skeleton().def;
    let shift = [Motion::Begin(pose), Motion::End(from)]
        .into_iter()
        .find(|motion| skeleton.plays(stance, motion))?;
    play(world, actor, shift, None)
}

/// Whether an actor's body has a clip for `motion` in its stance: one with none never gestures.
pub fn plays(world: &World, actor: Entity, motion: &Motion) -> bool {
    let content = world.resource::<Content>();
    world.get::<Appearance>(actor).is_some_and(|appearance| {
        ActorModel::get(content, appearance.model)
            .skeleton()
            .def
            .plays(held::stance_of(world, actor), motion)
    })
}

pub fn playing(world: &World, actor: Entity) -> bool {
    world.get::<Playing>(actor).is_some()
}

pub fn progress(world: &mut World, playing: &mut QueryState<Entity, With<Playing>>) {
    let now = world.resource::<WallClock>().now;
    let actors: Vec<Entity> = playing.iter(world).collect();
    for actor in actors {
        let Some(played) = world.get::<Playing>(actor).cloned() else {
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
            settle_awaited(world, actor, NEXT);
        }
    }
}

/// Whether the actor may begin another action: nothing playing, and the attack delay over.
pub fn ready(world: &World, actor: Entity) -> bool {
    if playing(world, actor) {
        return false;
    }
    let now = world.resource::<WallClock>().now;
    let delay = Millis(stat::effective(world, actor, StatKind::AttackDelay));
    world
        .get::<LastAction>(actor)
        .is_none_or(|last| now.since(last.at) >= delay.seconds())
}

fn interrupt(world: &mut World, actor: Entity) {
    world.entity_mut(actor).remove::<Playing>();
    settle_awaited(world, actor, INTERRUPTED);
}

/// An action's end answers the script still waiting on it: one that waited for the end, or one
/// that waited for an apex that never came.
fn settle_awaited(world: &mut World, actor: Entity, output: &'static str) {
    if let Some(awaited) = world.entity_mut(actor).take::<Awaited>()
        && (awaited.wait == Wait::End || !awaited.apex_answered)
    {
        answer(world, awaited.flow, output);
    }
}

fn answer(world: &mut World, flow: FlowKey, output: &'static str) {
    world
        .resource_mut::<RiftInstances>()
        .answer(flow, Answer::Output(output));
}
