use bevy_ecs::prelude::*;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use crate::core::content::Content;
use crate::core::math::{Pos, Rng};
use crate::core::par;
use crate::core::script::{
    Answer, Arrow, Compiler, EffectDef, EffectStep, Eval, FlowCx, ScriptFault, Value, ValueType,
};
use crate::core::tick::{Cached, Planner};
use crate::core::units::{MeterPos, Meters};
use crate::systems::actor::{self, Actor};
use crate::systems::area::{self, AreaTag, object};
use crate::systems::dialogue::ConversationSession;
use crate::systems::movement::{self, Approach, ApproachPlan, NavMesh};
use crate::systems::npc::{Home, Npc, Roams};
use crate::systems::script::event::NPC;
use crate::systems::script::{
    Condition, EffectCall, EffectLook, FlowKey, Place, Reach, RiftInstances, RiftScripting,
    RunnerKind, query,
};
use crate::systems::{dialogue, reach, skill, stat};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub u8);

impl Priority {
    pub const LOWEST: u8 = 1;
    pub const HIGHEST: u8 = 9;

    pub fn checked(priority: u8) -> Result<Priority, ScriptFault> {
        if (Priority::LOWEST..=Priority::HIGHEST).contains(&priority) {
            Ok(Priority(priority))
        } else {
            Err(ScriptFault::new(
                "out of range",
                format!(
                    "a priority {} to {}, not {priority}",
                    Priority::LOWEST,
                    Priority::HIGHEST
                ),
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Wander { leash: Option<Meters> },
    WalkTo { to: Destination, within: Meters },
    Attack { target: Entity, within: Meters },
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
    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Destination {
    Place(Pos<Meters>),
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
    if flow.is_none() && doing(world, npc, act, priority) {
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

/// Drives every npc's body on: what each does next is worked out on every core from the world as
/// the tick found it, then done in the order the npcs were found.
#[derive(Default)]
pub struct DriveBodies {
    bodies: Cached<Entity, With<Body>>,
    sessions: Cached<(Entity, &'static ConversationSession), ()>,
    drives: Vec<(Entity, Drive)>,
}

impl Planner for DriveBodies {
    fn prepare(&mut self, world: &mut World) {
        self.bodies.ready(world);
        self.sessions.ready(world);
    }

    fn plan(&mut self, world: &World) {
        let npcs: Vec<Entity> = self.bodies.query(world).iter().collect();
        let mut talking = dialogue::counterparts(self.sessions.query(world).iter());
        talking.sort_unstable();
        self.drives = npcs
            .par_iter()
            .with_min_len(par::grain(npcs.len()))
            .filter_map(|&npc| Some((npc, drive_of(world, npc, &talking)?)))
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        for (npc, drive) in self.drives.drain(..) {
            match drive {
                Drive::Halt => movement::halt(world, npc),
                Drive::Drop => {
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
                }
                Drive::Start => start_waiting(world, npc),
                Drive::Walk(current, plan) => {
                    let outcome = walk_on(world, npc, current, plan);
                    if !current.started
                        && let Some(mut body) = world.get_mut::<Body>(npc)
                        && let Some(running) = body.current.as_mut()
                    {
                        running.started = true;
                    }
                    if let Some(output) = outcome {
                        done(world, npc, current, output);
                    }
                }
                Drive::Step(current) => {
                    let outcome = step(world, npc, current);
                    if !current.started
                        && let Some(mut body) = world.get_mut::<Body>(npc)
                        && let Some(running) = body.current.as_mut()
                    {
                        running.started = true;
                    }
                    if let Some(output) = outcome {
                        done(world, npc, current, output);
                    }
                }
                Drive::Wander(ground) => {
                    wander_to(world, npc, ground);
                    if let Some(mut body) = world.get_mut::<Body>(npc)
                        && let Some(running) = body.current.as_mut()
                    {
                        running.started = true;
                    }
                }
                Drive::Done(current, output) => done(world, npc, current, output),
            }
        }
    }
}

/// Whether the npc's body is on `act` at `priority` already, with no flow waiting on it: an
/// order for the same is nothing new.
pub fn doing(world: &World, npc: Entity, act: Act, priority: Priority) -> bool {
    world
        .get::<Body>(npc)
        .and_then(|body| body.current)
        .is_some_and(|current| {
            !current.pauses() && current.act == act && current.priority == priority
        })
}

/// What an npc's body does this tick, as the tick found it.
enum Drive {
    /// Stands still to talk.
    Halt,
    /// Fallen: every act of its is lost.
    Drop,
    Start,
    /// Steps its current act, which may move it.
    Step(Running),
    /// Walks on toward where its current act leads, as planned from the world as the tick found it.
    Walk(Running, Option<ApproachPlan>),
    /// Sets out wandering, over the ground it may wander: triangles to draw a spot from.
    Wander(Vec<[Pos<Meters>; 3]>),
    /// Its current act came to an end on its own.
    Done(Running, Outcome),
}

fn drive_of(world: &World, npc: Entity, talking: &[Entity]) -> Option<Drive> {
    if talking.binary_search(&npc).is_ok() {
        return movement::moving(world, npc).then_some(Drive::Halt);
    }
    let body = world.get::<Body>(npc)?;
    if stat::is_dead(world, npc) {
        return (body.current.is_some() || !body.waiting.is_empty()).then_some(Drive::Drop);
    }
    let Some(current) = body.current else {
        return (!body.waiting.is_empty()).then_some(Drive::Start);
    };
    Some(match current.act {
        Act::Wander { .. } if current.started => {
            if movement::moving(world, npc) {
                return None;
            }
            Drive::Done(current, Outcome::Next)
        }
        Act::Wander { leash } => Drive::Wander(wander_ground(world, npc, leash)),
        Act::WalkTo { to, within } => Drive::Walk(
            current,
            destination(world, npc, to, within)
                .map(|aim| ApproachPlan::of(world, npc, aim, within)),
        ),
        Act::Attack { .. } | Act::Stop => Drive::Step(current),
    })
}

/// The ground a wander may lead over: within the leash of the npc's home, within the range it
/// roams, or within the leash of where it stands when it has neither. The leash is the wander's
/// own, else the npc's.
fn wander_ground(world: &World, npc: Entity, leash: Option<Meters>) -> Vec<[Pos<Meters>; 3]> {
    let content = world.resource::<Content>();
    let (Some(at), Some(mesh), Some(body)) = (
        movement::position(world, npc),
        movement::mesh_of(content, world, npc),
        world.get::<Npc>(npc),
    ) else {
        return Vec::new();
    };
    let leash = leash.unwrap_or(body.def.get(content).leash);
    let home = world.get::<Home>(npc).map(|home| home.0);
    let roams = world.get::<Roams>(npc).map(|roams| roams.0);
    match (home, roams) {
        (Some(home), _) => mesh.reachable_near(at, home, leash),
        (None, Some(roams)) => mesh.reachable_within(at, roams),
        (None, None) => mesh.reachable_near(at, at, leash),
    }
}

/// Sends the npc wandering to a free spot on `ground`.
fn wander_to(world: &mut World, npc: Entity, ground: Vec<[Pos<Meters>; 3]>) {
    let goal = world.resource_scope(|world, mut rng: Mut<Rng>| {
        let free = |spot| movement::crowding(world, npc, spot) == 0;
        NavMesh::draw_on(&mut rng, ground, free)
    });
    if let Some(goal) = goal {
        movement::goto(world, npc, goal);
    }
}

/// Ends the npc's current act with its outcome, and takes up the next waiting.
fn done(world: &mut World, npc: Entity, current: Running, output: Outcome) {
    reach::forget(world, npc);
    if let Some(mut body) = world.get_mut::<Body>(npc) {
        body.current = None;
    }
    answer(world, current, output);
    start_waiting(world, npc);
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
        Act::Wander { .. } | Act::WalkTo { .. } => {
            (!movement::moving(world, npc)).then_some(Outcome::Next)
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
            if !casting
                && skill::cast(world, npc, skill, skill::CastAim::Actor(target), true).is_err()
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

fn walk_on(
    world: &mut World,
    npc: Entity,
    running: Running,
    plan: Option<ApproachPlan>,
) -> Option<Outcome> {
    let Some(plan) = plan else {
        return Some(Outcome::Blocked);
    };
    match plan.apply(world, npc) {
        Approach::Arrived => Some(Outcome::Arrived),
        Approach::Unreachable => Some(Outcome::Blocked),
        Approach::Approaching => {
            (running.started && !movement::moving(world, npc)).then_some(Outcome::Blocked)
        }
    }
}

/// A place or object stood on by others is walked to beside them instead.
fn destination(
    world: &World,
    npc: Entity,
    to: Destination,
    within: Meters,
) -> Option<movement::Aim> {
    let spot = match to {
        Destination::Place(at) => at,
        Destination::Object(object) => object::shape_of(world, object)?.center(),
        Destination::Actor(actor) => {
            let there = !stat::is_dead(world, actor) && area::same_area(world, npc, actor);
            return there.then_some(movement::Aim::Actor(actor));
        }
    };
    Some(movement::Aim::Point(movement::vacant_near(
        world, npc, spot, within,
    )))
}

/// Walks to a random spot within `leash` of the npc's home, the npc's own leash when none is
/// given, or wherever it roams.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct Wander {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-value" = "number"))]
    pub leash: Option<Condition>,
    #[schemars(extend("minimum" = Priority::LOWEST, "maximum" = Priority::HIGHEST))]
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Walks to a place, an object or an actor.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct WalkTo {
    pub to: WalkTarget,
    #[schemars(extend("default" = 1.0))]
    pub within: Condition,
    #[schemars(extend("minimum" = Priority::LOWEST, "maximum" = Priority::HIGHEST))]
    pub priority: u8,
    /// Taken on getting there.
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub arrived: Arrow,
    /// Taken when no way leads there.
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub blocked: Arrow,
    /// Taken when another act of at least its priority takes the body.
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub interrupted: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WalkTarget {
    Place(Place),
    Object(String),
    #[schemars(extend("default" = {"type": "subject"}))]
    Actor(Condition),
}

/// Chases and swings at a target.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct Attack {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub target: Condition,
    #[schemars(extend("default" = 1.5))]
    pub within: Condition,
    #[schemars(extend("minimum" = Priority::LOWEST, "maximum" = Priority::HIGHEST))]
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub defeated: Arrow,
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub lost: Arrow,
    /// Taken when another act of at least its priority takes the body.
    #[schemars(extend("x-fails" = true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub interrupted: Arrow,
}

/// Interrupts the body.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "npc body"))]
pub struct Stop {
    #[schemars(extend("minimum" = Priority::LOWEST, "maximum" = Priority::HIGHEST))]
    pub priority: u8,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

/// Turns an actor toward a place, an actor or a point: the owner when it is an actor such as an
/// npc, else the subject, unless `who` names someone.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(extend("x-system" = "actor"))]
pub struct FaceToward {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Condition>,
    pub toward: FaceTarget,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FaceTarget {
    Place(Place),
    #[schemars(extend("default" = {"type": "subject"}))]
    Actor(Condition),
}

type Aimed<T> = Box<dyn Fn(&World, &FlowCx) -> Option<T> + Send + Sync>;

impl EffectDef<RiftScripting> for Wander {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(Outcome::Next.name() => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        let leash = cx.opt_expr(self.leash.as_ref(), ValueType::Number)?;
        Ok(Box::new(move |world, _, cx| {
            let leash = leash
                .as_ref()
                .map(|leash| Meters(leash(world, cx).as_num() as f32));
            if let Some(npc) = cx.owner {
                order(world, npc, Act::Wander { leash }, priority, None);
            }
            EffectStep::Done
        }))
    }
    fn look<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<Option<EffectLook>, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        let leash = cx.opt_expr(self.leash.as_ref(), ValueType::Number)?;
        Ok(Some(Box::new(move |world, _, cx| {
            let leash = leash
                .as_ref()
                .map(|leash| Meters(leash(world, cx).as_num() as f32));
            let npc = cx.owner?;
            doing(world, npc, Act::Wander { leash }, priority).then_some(EffectStep::Done)
        })))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Walks]
    }
}

impl EffectDef<RiftScripting> for WalkTo {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(Outcome::Arrived.name() => arrived, Outcome::Blocked.name() => blocked, Outcome::Interrupted.name() => interrupted);
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
            let within = Meters(within(world, cx).as_num() as f32);
            order(world, npc, Act::WalkTo { to, within }, priority, Some(flow));
            EffectStep::Pause
        }))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        vec![Reach::Walks]
    }
}

impl EffectDef<RiftScripting> for Attack {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(Outcome::Defeated.name() => defeated, Outcome::Lost.name() => lost, Outcome::Interrupted.name() => interrupted);
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
            let within = Meters(within(world, cx).as_num() as f32);
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
        vec![Reach::Targets]
    }
}

impl EffectDef<RiftScripting> for Stop {
    fn owners(&self) -> &'static [RunnerKind] {
        NPC
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(Outcome::Next.name() => next);
    fn compile(&self, _cx: &mut Compiler<RiftScripting>) -> Result<EffectCall, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        Ok(Box::new(move |world, _, cx| {
            if let Some(npc) = cx.owner {
                order(world, npc, Act::Stop, priority, None);
            }
            EffectStep::Done
        }))
    }
    fn look(&self, _cx: &mut Compiler<RiftScripting>) -> Result<Option<EffectLook>, ScriptFault> {
        let priority = Priority::checked(self.priority)?;
        Ok(Some(Box::new(move |world, _, cx| {
            let npc = cx.owner?;
            doing(world, npc, Act::Stop, priority).then_some(EffectStep::Done)
        })))
    }
    fn reach(&self, _cx: &Compiler<RiftScripting>) -> Vec<Reach> {
        Vec::new()
    }
}

impl EffectDef<RiftScripting> for FaceToward {
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(Outcome::Next.name() => next);
    fn compile<'c>(
        &'c self,
        cx: &mut Compiler<'c, RiftScripting>,
    ) -> Result<EffectCall, ScriptFault> {
        let toward: Aimed<Pos<Meters>> = match &self.toward {
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
