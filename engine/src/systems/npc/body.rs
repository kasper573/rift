use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::core::content::Content;
use crate::core::math::{Pos, Rng};
use crate::core::script::{
    Answer, Compiler, EffectDef, EffectStep, Eval, FlowCx, Name, ScriptFault, Value, ValueType,
};
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::actor::{self, Actor};
use crate::systems::area::{self, AreaTag, object};
use crate::systems::dialogue;
use crate::systems::movement;
use crate::systems::npc::{Home, Npc, Roams};
use crate::systems::reach;
use crate::systems::script::effect::{Aim, Arrow, Link};
use crate::systems::script::{
    Condition, EffectCall, FlowKey, OwnerKind, Place, Reach, RiftInstances, RiftScripting, query,
};
use crate::systems::skill;
use crate::systems::stat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub u8);

impl Priority {
    pub fn checked(priority: u8) -> Result<Priority, ScriptFault> {
        if (1..=9).contains(&priority) {
            Ok(Priority(priority))
        } else {
            Err(ScriptFault::new(
                "out of range",
                format!("a priority 1 to 9, not {priority}"),
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Wander { leash: Option<Tiles> },
    WalkTo { to: Destination, within: Tiles },
    Attack { target: Entity, within: Tiles },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Outcome {
    Next,
    Arrived,
    Blocked,
    Defeated,
    Lost,
    Interrupted,
}

impl Outcome {
    pub fn name(self) -> Name {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Destination {
    Place(Pos<Tiles>),
    Object(Entity),
    Actor(Entity),
}

#[derive(Component, Default)]
pub struct Body {
    current: Option<Running>,
    waiting: Vec<Running>,
}

#[derive(Clone, Copy)]
struct Running {
    act: Act,
    priority: Priority,
    flow: Option<FlowKey>,
    started: bool,
}

impl Running {
    fn pauses(&self) -> bool {
        matches!(self.act, Act::WalkTo { .. } | Act::Attack { .. })
    }
}

pub fn order(world: &mut World, npc: Entity, act: Act, priority: Priority, flow: Option<FlowKey>) {
    let running = Running {
        act,
        priority,
        flow,
        started: false,
    };
    let same_as_current = world
        .get::<Body>(npc)
        .and_then(|body| body.current)
        .is_some_and(|current| {
            !current.pauses() && current.act == act && current.priority == priority
        });
    if same_as_current && flow.is_none() {
        return;
    }
    let interrupted = {
        let Some(mut body) = world.get_mut::<Body>(npc) else {
            world.entity_mut(npc).insert(Body {
                current: Some(running),
                waiting: Vec::new(),
            });
            return;
        };
        match body.current {
            Some(current) if current.priority > priority => {
                body.waiting.push(running);
                body.waiting
                    .sort_by_key(|running| std::cmp::Reverse(running.priority));
                return;
            }
            current => {
                body.current = Some(running);
                current
            }
        }
    };
    if let Some(interrupted) = interrupted {
        reach::forget(world, npc);
        answer(world, interrupted, Outcome::Interrupted);
    }
}

pub fn target(world: &World, npc: Entity) -> Option<Entity> {
    match world.get::<Body>(npc)?.current?.act {
        Act::Attack { target, .. } => Some(target),
        _ => None,
    }
}

pub fn drive(
    world: &mut World,
    bodies: &mut QueryState<Entity, With<Body>>,
    sessions: &mut dialogue::Sessions,
    mut npcs: Local<Vec<Entity>>,
) {
    npcs.clear();
    npcs.extend(bodies.iter(world));
    let talking = dialogue::counterparts(world, sessions);
    for &npc in npcs.iter() {
        if talking.contains(&npc) {
            if movement::moving(world, npc) {
                movement::halt(world, npc);
            }
            continue;
        }
        if stat::is_dead(world, npc) {
            let Some(mut body) = world.get_mut::<Body>(npc) else {
                continue;
            };
            let dropped: Vec<Running> = body
                .current
                .take()
                .into_iter()
                .chain(body.waiting.drain(..))
                .collect();
            for running in dropped {
                answer(world, running, Outcome::Lost);
            }
            continue;
        }
        let Some(current) = world.get::<Body>(npc).and_then(|body| body.current) else {
            start_waiting(world, npc);
            continue;
        };
        let outcome = step(world, npc, current);
        if !current.started
            && let Some(mut body) = world.get_mut::<Body>(npc)
            && let Some(running) = body.current.as_mut()
        {
            running.started = true;
        }
        if let Some(output) = outcome {
            reach::forget(world, npc);
            if let Some(mut body) = world.get_mut::<Body>(npc) {
                body.current = None;
            }
            answer(world, current, output);
            start_waiting(world, npc);
        }
    }
}

fn start_waiting(world: &mut World, npc: Entity) {
    let Some(mut body) = world.get_mut::<Body>(npc) else {
        return;
    };
    if body.current.is_none() && !body.waiting.is_empty() {
        body.current = Some(body.waiting.remove(0));
    }
}

fn answer(world: &mut World, running: Running, outcome: Outcome) {
    if running.pauses()
        && let Some(flow) = running.flow
    {
        world
            .resource_mut::<RiftInstances>()
            .answer(flow, Answer::Output(outcome.name()));
    }
}

fn step(world: &mut World, npc: Entity, running: Running) -> Option<Outcome> {
    match running.act {
        Act::Stop => {
            movement::halt(world, npc);
            Some(Outcome::Next)
        }
        Act::Wander { leash } => {
            if !running.started {
                wander(world, npc, leash);
                return None;
            }
            (!movement::moving(world, npc)).then_some(Outcome::Next)
        }
        Act::WalkTo { to, within } => {
            let Some(goal) = destination(world, npc, to) else {
                return Some(Outcome::Blocked);
            };
            if movement::approach(world, npc, goal, within) {
                movement::halt(world, npc);
                return Some(Outcome::Arrived);
            }
            if running.started && !movement::moving(world, npc) {
                return Some(Outcome::Blocked);
            }
            None
        }
        Act::Attack { target, within } => {
            if world.get_entity(target).is_err() || stat::is_dead(world, target) {
                return Some(Outcome::Defeated);
            }
            let Some(skill) = world
                .get::<Npc>(npc)
                .and_then(|body| body.def.get(world.resource::<Content>()).basic_attack)
            else {
                return Some(Outcome::Lost);
            };
            let casting = reach::current(world, npc).is_some_and(|intent| {
                intent.target.actor() == Some(target) && intent.cast() == Some(skill)
            });
            if !casting && skill::cast(world, npc, skill, skill::Aim::Actor(target), true).is_err()
            {
                return Some(Outcome::Lost);
            }
            let lost = (|| {
                let at = movement::position(world, npc)?;
                let there = movement::position(world, target)?;
                let sheltered = area::wild(world, npc)
                    && area::of(world, npc).is_some_and(|area| area.safe(there));
                Some(
                    !reach::pursuable(world, npc, target)
                        || at.distance(there) > within
                        || sheltered,
                )
            })()
            .unwrap_or(true);
            if lost {
                return Some(Outcome::Lost);
            }
            None
        }
    }
}

fn destination(world: &World, npc: Entity, to: Destination) -> Option<Pos<Tiles>> {
    match to {
        Destination::Place(at) => Some(at),
        Destination::Object(object) => object::shape_of(world, object).map(|shape| shape.center()),
        Destination::Actor(actor) => {
            if stat::is_dead(world, actor) {
                return None;
            }
            if !area::same_area(world, npc, actor) {
                return None;
            }
            movement::position(world, actor)
        }
    }
}

fn wander(world: &mut World, npc: Entity, leash: Option<Tiles>) {
    let Some(at) = movement::position(world, npc) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let Some(area) = area::of_in(&content, world, npc) else {
        return;
    };
    let grid = area.grid_for(area::wild(world, npc));
    let home = world.get::<Home>(npc).map(|home| home.0);
    let roams = world.get::<Roams>(npc).map(|roams| roams.0);
    let goal = {
        let mut rng = world.resource_mut::<Rng>();
        match (leash.zip(home), roams) {
            (Some((leash, home)), _) => grid.random_reachable_near(&mut rng, at, home, leash),
            (None, Some(roams)) => grid.random_reachable_within(&mut rng, at, roams),
            (None, None) => grid.random_reachable(&mut rng, at),
        }
    };
    if let Some(goal) = goal {
        movement::goto(world, npc, goal);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Wander {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leash: Option<Condition>,
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct WalkTo {
    pub to: WalkTarget,
    pub within: Condition,
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrived: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WalkTarget {
    Place(Place),
    Object(String),
    Actor(Condition),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Attack {
    pub target: Condition,
    pub within: Condition,
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defeated: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Arrow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Stop {
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

/// Turns an actor toward a place, an actor or a point: the owner when it is an actor such as an
/// npc, else the subject, unless `who` names someone.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct FaceToward {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Aim,
    pub toward: FaceTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FaceTarget {
    Place(Place),
    Actor(Condition),
}

type Aimed<T> = Box<dyn Fn(&World, &FlowCx) -> Option<T> + Send + Sync>;

const NPC: &[OwnerKind] = &[OwnerKind::Npc];

impl EffectDef<RiftScripting> for Wander {
    fn owners(&self) -> &'static [OwnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(Outcome::Next.name(), self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        let leash = cx.opt_expr(self.leash.as_ref(), ValueType::Number)?;
        Ok(Box::new(move |world, _, cx| {
            let leash = leash
                .as_ref()
                .map(|leash| Tiles(leash(world, cx).as_num() as f32));
            if let Some(npc) = cx.owner {
                order(world, npc, Act::Wander { leash }, priority, None);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Walks]
    }
}

impl EffectDef<RiftScripting> for WalkTo {
    fn owners(&self) -> &'static [OwnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![
            (Outcome::Arrived.name(), self.arrived.as_deref()),
            (Outcome::Blocked.name(), self.blocked.as_deref()),
            (Outcome::Interrupted.name(), self.interrupted.as_deref()),
        ]
    }
    fn pauses(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        let within = cx.expr(&self.within, ValueType::Number)?;
        let to: Aimed<Destination> = match &self.to {
            WalkTarget::Place(place) => {
                let at = place.at;
                Box::new(move |_, _| Some(Destination::Place(at)))
            }
            WalkTarget::Object(name) => {
                let name = name.clone();
                Box::new(move |world, cx| {
                    let area = cx.owner.and_then(|npc| world.get::<AreaTag>(npc))?.area;
                    object::named(world, area, &name).map(Destination::Object)
                })
            }
            WalkTarget::Actor(actor) => {
                let actor: Eval = cx.expr(actor, ValueType::Actor)?;
                Box::new(move |world, cx| query::actor(world, cx, &actor).map(Destination::Actor))
            }
        };
        Ok(Box::new(move |world, flow, cx| {
            let Some(npc) = cx.owner else {
                return EffectStep::Output(Outcome::Blocked.name());
            };
            let Some(to) = to(world, cx) else {
                return EffectStep::Output(Outcome::Blocked.name());
            };
            let within = Tiles(within(world, cx).as_num() as f32);
            order(world, npc, Act::WalkTo { to, within }, priority, Some(flow));
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Walks]
    }
}

impl EffectDef<RiftScripting> for Attack {
    fn owners(&self) -> &'static [OwnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![
            (Outcome::Defeated.name(), self.defeated.as_deref()),
            (Outcome::Lost.name(), self.lost.as_deref()),
            (Outcome::Interrupted.name(), self.interrupted.as_deref()),
        ]
    }
    fn pauses(&self) -> bool {
        true
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        let target = cx.expr(&self.target, ValueType::Actor)?;
        let within = cx.expr(&self.within, ValueType::Number)?;
        Ok(Box::new(move |world, flow, cx| {
            let Some(npc) = cx.owner else {
                return EffectStep::Output(Outcome::Lost.name());
            };
            let Some(target) = query::actor(world, cx, &target) else {
                return EffectStep::Output(Outcome::Lost.name());
            };
            let within = Tiles(within(world, cx).as_num() as f32);
            order(
                world,
                npc,
                Act::Attack { target, within },
                priority,
                Some(flow),
            );
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![crate::systems::script::Reach::Targets]
    }
}

impl EffectDef<RiftScripting> for Stop {
    fn owners(&self) -> &'static [OwnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(Outcome::Next.name(), self.next.as_deref())]
    }
    fn compile(&self, _cx: &mut Compiler<RiftScripting>) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        Ok(Box::new(move |world, _, cx| {
            if let Some(npc) = cx.owner {
                order(world, npc, Act::Stop, priority, None);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        Vec::new()
    }
}

impl EffectDef<RiftScripting> for FaceToward {
    fn needs_subject(&self) -> bool {
        false
    }
    fn outputs(&self) -> Vec<(Name, Link<'_>)> {
        vec![(Outcome::Next.name(), self.next.as_deref())]
    }
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let toward: Aimed<Pos<Tiles>> = match &self.toward {
            FaceTarget::Place(place) => {
                let at = place.at;
                Box::new(move |_, _| Some(at))
            }
            FaceTarget::Actor(target) => {
                let (target, ty) = cx.typed(target)?;
                if !matches!(ty, ValueType::Actor | ValueType::Point) {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("toward is {}, expected an actor or a point", ty.name()),
                    ));
                }
                Box::new(move |world, cx| match target(world, cx) {
                    Value::Actor(Some(who)) => movement::position(world, who),
                    Value::Point(point) => point,
                    _ => None,
                })
            }
        };
        let who = self
            .who
            .as_ref()
            .map(|who| cx.expr(who, ValueType::Actor))
            .transpose()?;
        Ok(Box::new(move |world, _, cx| {
            let turning = match &who {
                Some(who) => query::actor(world, cx, who),
                None => cx
                    .owner
                    .filter(|owner| world.get::<Actor>(*owner).is_some())
                    .or(cx.subject),
            };
            if let (Some(turning), Some(at)) = (turning, toward(world, cx)) {
                actor::face_toward(world, turning, at);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        Vec::new()
    }
}
