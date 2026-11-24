use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_time::Time;
use rayon::prelude::*;
use slotmap::{SlotMap, new_key_type};
use smallvec::{SmallVec, smallvec};

use super::compile::{EffectStep, Procedure, ProcedureId, Slot, Step, StepIndex};
use super::dialogue::{ChoiceOffer, DialogueView};
use super::may_do::MayDo;
use super::node::MAX_LIVE_FLOWS;
use super::value::Value;
use super::vocabulary::Vocabulary;
use super::{EventKind, RunnerKind, ScriptEvent};
use crate::core::math::Rng;
use crate::core::par;
use crate::core::tick::Planner;

/// Flows that keep waking one another stop for the tick here, so they cannot hold the server.
const MAX_ROUNDS_PER_TICK: u32 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Entity(RunnerKind, Entity),
    Row(RunnerKind, u32),
}

impl Owner {
    pub fn kind(self) -> RunnerKind {
        match self {
            Owner::Entity(kind, _) | Owner::Row(kind, _) => kind,
        }
    }

    pub fn entity(self) -> Option<Entity> {
        match self {
            Owner::Entity(_, entity) => Some(entity),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub owner: Owner,
    pub flow: FlowId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlowId(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pause {
    Wait { until: Duration },
    NextTick { tick: u64 },
    Step { tick: u64 },
    Dialogue,
    Effect { keeps_box: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Output(&'static str),
    Pick(usize),
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxEnding {
    Ended,
    Left,
    Gone,
}

pub type Locals = SmallVec<[Value; 2]>;

#[derive(Clone)]
pub struct FlowCx {
    pub owner: Option<Entity>,
    pub subject: Option<Entity>,
    pub locals: Locals,
    pub state: Vec<Value>,
    pub event: Vec<Value>,
}

impl FlowCx {
    /// A reading of the world with nobody as its subject.
    pub fn nobody() -> FlowCx {
        FlowCx {
            owner: None,
            subject: None,
            locals: Locals::new(),
            state: Vec::new(),
            event: Vec::new(),
        }
    }

    pub fn standalone(subject: Entity) -> FlowCx {
        FlowCx {
            owner: None,
            subject: Some(subject),
            locals: Locals::new(),
            state: Vec::new(),
            event: Vec::new(),
        }
    }

    pub fn event_field(&self, index: usize) -> Value {
        self.event[index].clone()
    }
}

type Accept = Box<dyn Fn(&ScriptEvent) -> bool + Send + Sync>;

enum Op {
    Spawn {
        owner: Owner,
        procedures: Vec<Arc<Procedure>>,
    },
    Remove {
        owner: Owner,
    },
    Raise {
        owner: Owner,
        kind: EventKind,
        subject: Option<Entity>,
        event: Vec<Value>,
        accept: Accept,
    },
    Answer {
        key: FlowKey,
        answer: Answer,
    },
    BoxClosed {
        key: FlowKey,
    },
}

#[derive(Resource)]
pub struct Instances {
    instances: HashMap<Owner, Instance>,
    answering: HashMap<EventKind, Vec<Owner>>,
    ops: VecDeque<Op>,
    queue: VecDeque<Queued>,
    flows: SlotMap<FlowSlot, Option<Box<Flow>>>,
    by_id: HashMap<FlowId, FlowSlot>,
    next_flow: FlowId,
    tick: u64,
}

new_key_type! {
    struct FlowSlot;
}

impl Default for Instances {
    fn default() -> Self {
        Instances {
            instances: HashMap::new(),
            answering: HashMap::new(),
            ops: VecDeque::new(),
            queue: VecDeque::new(),
            flows: SlotMap::with_key(),
            by_id: HashMap::new(),
            next_flow: FlowId(1),
            tick: 0,
        }
    }
}

enum Queued {
    Start {
        owner: Owner,
        procedure: usize,
        entry: StepIndex,
        subject: Option<Entity>,
        event: Vec<Value>,
    },
    Resume {
        key: FlowKey,
        answer: Answer,
    },
}

struct Instance {
    procedures: Vec<Arc<Procedure>>,
    may_do: MayDo,
    state: OwnerState,
    flows: Vec<FlowSlot>,
}

/// The state of an owner's procedures, by procedure.
type OwnerState = HashMap<ProcedureId, Vec<Value>>;

struct Flow {
    owner: Owner,
    id: FlowId,
    procedure: usize,
    entry: StepIndex,
    subject: Option<Entity>,
    event: Vec<Value>,
    frames: SmallVec<[Frame; 1]>,
    pause: Option<Pause>,
    box_open: bool,
    /// Picked to run in the batch under way.
    running: bool,
    /// The flow's own dice, forked from the world's when it started, so its rolls read nothing
    /// but the flow.
    dice: Rng,
}

struct Frame {
    procedure: Arc<Procedure>,
    node: Option<StepIndex>,
    locals: Locals,
    loops: Vec<(StepIndex, u32)>,
    into: Option<usize>,
    return_to: Option<StepIndex>,
}

impl Instances {
    pub fn spawn_owner(&mut self, owner: Owner, procedures: Vec<Arc<Procedure>>) {
        self.ops.push_back(Op::Spawn { owner, procedures });
    }

    pub fn remove_owner(&mut self, owner: Owner) {
        self.ops.push_back(Op::Remove { owner });
    }

    pub fn raise(
        &mut self,
        owner: Owner,
        kind: EventKind,
        subject: Option<Entity>,
        event: Vec<Value>,
        accept: impl Fn(&ScriptEvent) -> bool + Send + Sync + 'static,
    ) {
        self.ops.push_back(Op::Raise {
            owner,
            kind,
            subject,
            event,
            accept: Box::new(accept),
        });
    }

    pub fn answer(&mut self, key: FlowKey, answer: Answer) {
        self.ops.push_back(Op::Answer { key, answer });
    }

    pub fn box_closed(&mut self, key: FlowKey) {
        self.ops.push_back(Op::BoxClosed { key });
    }

    pub fn may_do(&self, owner: Owner) -> Option<&MayDo> {
        self.instances.get(&owner).map(|instance| &instance.may_do)
    }

    pub fn owners_answering(&self, kind: EventKind) -> &[Owner] {
        self.answering.get(&kind).map_or(&[], Vec::as_slice)
    }

    pub fn answers(&self, owner: Owner, kind: EventKind) -> bool {
        self.instances
            .get(&owner)
            .is_some_and(|instance| instance.may_do.answers(kind))
    }

    pub fn entries(&self, owner: Owner, kind: EventKind) -> impl Iterator<Item = &ScriptEvent> {
        self.instances
            .get(&owner)
            .into_iter()
            .flat_map(|instance| instance.procedures.iter())
            .flat_map(move |procedure| {
                procedure
                    .entries
                    .iter()
                    .filter(move |(_, event)| event.kind() == kind)
                    .map(|(_, event)| event)
            })
    }

    pub fn next_dialogue(
        &self,
        world: &World,
        owner: Owner,
        kind: EventKind,
        subject: Entity,
    ) -> Vec<(String, String)> {
        let Some(instance) = self.instances.get(&owner) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for procedure in &instance.procedures {
            for (entry, _) in procedure.entry(kind) {
                let cx = fresh_cx(instance, procedure, owner.entity(), Some(subject));
                peek_dialogues(world, instance, procedure, entry, cx, &mut found);
            }
        }
        found
    }

    pub fn pause_of(&self, key: FlowKey) -> Option<Pause> {
        self.flow(key).and_then(|flow| flow.pause)
    }

    pub fn live_flows(&self, owner: Owner) -> usize {
        self.instances
            .get(&owner)
            .map_or(0, |instance| instance.flows.len())
    }

    fn state(&self, owner: Owner) -> Option<&OwnerState> {
        self.instances.get(&owner).map(|instance| &instance.state)
    }

    fn flow(&self, key: FlowKey) -> Option<&Flow> {
        let slot = *self.by_id.get(&key.flow)?;
        self.flows.get(slot)?.as_deref()
    }

    fn flow_mut(&mut self, key: FlowKey) -> Option<&mut Flow> {
        let slot = *self.by_id.get(&key.flow)?;
        self.flows.get_mut(slot)?.as_deref_mut()
    }

    fn set_state(&mut self, owner: Owner, procedure: ProcedureId, state: Vec<Value>) {
        if let Some(instance) = self.instances.get_mut(&owner)
            && instance.state.get(&procedure) != Some(&state)
        {
            instance.state.insert(procedure, state);
        }
    }

    fn start_flow(
        &mut self,
        owner: Owner,
        procedure: usize,
        entry: StepIndex,
        subject: Option<Entity>,
        event: Vec<Value>,
        dice: Rng,
    ) -> bool {
        let Some(instance) = self.instances.get_mut(&owner) else {
            return false;
        };
        let id = self.next_flow;
        self.next_flow = FlowId(id.0 + 1);
        let root = instance.procedures[procedure].clone();
        let locals = root.locals.iter().map(|(_, ty)| ty.zero()).collect();
        let event = padded(event, &root, entry);
        let flow = Flow {
            owner,
            id,
            procedure,
            entry,
            subject,
            event,
            frames: smallvec![Frame {
                procedure: root,
                node: Some(entry),
                locals,
                loops: Vec::new(),
                into: None,
                return_to: None,
            }],
            pause: None,
            box_open: false,
            running: true,
            dice,
        };
        let slot = self.flows.insert(Some(Box::new(flow)));
        self.by_id.insert(id, slot);
        instance.flows.push(slot);
        true
    }

    fn is_full(&self, owner: Owner) -> bool {
        self.instances
            .get(&owner)
            .is_some_and(|instance| instance.flows.len() >= MAX_LIVE_FLOWS)
    }

    fn drop_flow(&mut self, key: FlowKey) {
        let Some(slot) = self.by_id.remove(&key.flow) else {
            return;
        };
        self.flows.remove(slot);
        if let Some(instance) = self.instances.get_mut(&key.owner) {
            instance.flows.retain(|kept| *kept != slot);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceState {
    Open,
    Locked,
    Gone,
}

pub fn choice_state(world: &mut World, key: FlowKey, choice: usize) -> ChoiceState {
    match choice_open(&*world, key, choice) {
        Some(true) => ChoiceState::Open,
        Some(false) => ChoiceState::Locked,
        None => ChoiceState::Gone,
    }
}

fn choice_open(world: &World, key: FlowKey, choice: usize) -> Option<bool> {
    let instances = world.resource::<Instances>();
    let flow = instances.flow(key)?;
    if flow.pause != Some(Pause::Dialogue) {
        return None;
    }
    let top = flow.frames.last()?;
    let Step::Dialogue { requires, .. } = top.procedure.step(top.node?) else {
        return None;
    };
    let cx = cx_clone(key, flow, instances.state(key.owner));
    requires.get(choice).map(|requires| {
        requires
            .as_ref()
            .is_none_or(|requires| requires(world, &cx).truthy())
    })
}

/// Runs the flows due each tick: every one that wakes reads ahead on every core, through the
/// steps that only read the world as the tick found it; then each finishes in turn, by flow, its
/// steps that write the world applied and read on from; then whatever those writes start or
/// resume runs on, round by round, until nothing is left to run.
pub struct FlowRunner {
    flows: SlotMap<FlowSlot, Option<Box<Flow>>>,
    reads: Vec<Read>,
    clock: Option<TickClock>,
    steering: Steering,
}

impl Default for FlowRunner {
    fn default() -> FlowRunner {
        FlowRunner {
            flows: SlotMap::with_key(),
            reads: Vec::new(),
            clock: None,
            steering: Steering::default(),
        }
    }
}

impl Planner for FlowRunner {
    fn prepare(&mut self, world: &mut World) {
        self.clock = None;
        let Some(mut instances) = world.get_resource_mut::<Instances>() else {
            return;
        };
        instances.tick += 1;
        let tick = instances.tick;
        self.flows = std::mem::take(&mut instances.flows);
        self.clock = Some(TickClock {
            tick,
            now: world.resource::<Time>().elapsed(),
        });
        self.steering = Steering::of(world);
    }

    fn plan(&mut self, world: &World) {
        self.reads.clear();
        let Some(clock) = self.clock else {
            return;
        };
        wake_due(world, &mut self.flows, clock);
        let owners = &world.resource::<Instances>().instances;
        self.reads = read_batch(world, owners, &mut self.flows, clock, self.steering);
    }

    fn apply(&mut self, world: &mut World) {
        let Some(clock) = self.clock.take() else {
            return;
        };
        std::mem::swap(
            &mut world.resource_mut::<Instances>().flows,
            &mut self.flows,
        );
        finish_reads(world, std::mem::take(&mut self.reads), clock, self.steering);
        let mut rounds = 0;
        loop {
            apply_ops(world);
            if !pick_queued(world) {
                break;
            }
            run_batch(world, clock);
            rounds += 1;
            if rounds > MAX_ROUNDS_PER_TICK {
                bevy::log::warn!("flows kept waking one another; stopped for this tick");
                break;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct TickClock {
    tick: u64,
    now: Duration,
}

/// Readies every flow whose wait is over: at the step after its wait, or back at the step it
/// stepped to. A disabled owner is asleep: its flows wait, due, until it wakes. An owner that
/// has left the world runs no more of them: they wait for its instance to be closed.
fn wake_due(world: &World, flows: &mut SlotMap<FlowSlot, Option<Box<Flow>>>, clock: TickClock) {
    for flow in flows.values_mut().flatten() {
        let due = match flow.pause {
            Some(Pause::Wait { until }) => until <= clock.now,
            Some(Pause::NextTick { tick }) | Some(Pause::Step { tick }) => tick <= clock.tick,
            _ => false,
        };
        if !due {
            continue;
        }
        let asleep_or_gone = flow.owner.entity().is_some_and(|owner| {
            world
                .get_entity(owner)
                .map_or(true, |owner| owner.contains::<Disabled>())
        });
        if asleep_or_gone {
            continue;
        }
        let Some(top) = flow.frames.last_mut() else {
            continue;
        };
        let Some(node) = top.node else {
            continue;
        };
        match (top.procedure.step(node), flow.pause) {
            (Step::Wait { next, .. }, Some(Pause::Wait { .. } | Pause::NextTick { .. })) => {
                top.node = *next;
            }
            (_, Some(Pause::Step { .. })) => {}
            _ => continue,
        }
        flow.pause = None;
        flow.running = true;
    }
}

/// Readies every flow queued to run: the ones starting, and the ones resumed by the answer they
/// waited for. Tells whether any is.
fn pick_queued(world: &mut World) -> bool {
    let mut any = false;
    while let Some(queued) = world.resource_mut::<Instances>().queue.pop_front() {
        match queued {
            Queued::Start {
                owner,
                procedure,
                entry,
                subject,
                event,
            } => {
                if world.resource::<Instances>().is_full(owner) {
                    bevy::log::warn!("{owner:?}: flow dropped: {MAX_LIVE_FLOWS} live flows");
                    continue;
                }
                let dice = world.resource_mut::<Rng>().fork();
                any |= world
                    .resource_mut::<Instances>()
                    .start_flow(owner, procedure, entry, subject, event, dice);
            }
            Queued::Resume { key, answer } => any |= resume(world, key, answer),
        }
    }
    any
}

/// Runs the flows picked: first every one reads ahead on every core, through the steps that
/// only read the world; then each finishes in turn, by flow, its steps that write the world
/// applied and read on from. What a flow reads before its first write is the world as the
/// batch found it.
fn run_batch(world: &mut World, clock: TickClock) {
    let steering = Steering::of(world);
    let mut flows = std::mem::take(&mut world.resource_mut::<Instances>().flows);
    let reads = read_batch(
        world,
        &world.resource::<Instances>().instances,
        &mut flows,
        clock,
        steering,
    );
    world.resource_mut::<Instances>().flows = flows;
    finish_reads(world, reads, clock, steering);
}

/// Every running flow read ahead on every core, through the steps that only read the world.
fn read_batch(
    world: &World,
    owners: &HashMap<Owner, Instance>,
    flows: &mut SlotMap<FlowSlot, Option<Box<Flow>>>,
    clock: TickClock,
    steering: Steering,
) -> Vec<Read> {
    let running: Vec<(FlowSlot, &mut Flow)> = flows
        .iter_mut()
        .filter_map(|(slot, flow)| Some((slot, &mut **flow.as_mut()?)))
        .filter(|(_, flow)| flow.running)
        .collect();
    let read = |(slot, flow): (FlowSlot, &mut Flow)| {
        let key = FlowKey {
            owner: flow.owner,
            flow: flow.id,
        };
        let mut trace = Vec::new();
        let stopped = read_ahead(world, owners, key, flow, clock, steering, &mut trace);
        Read {
            slot,
            key,
            stopped,
            trace,
        }
    };
    let mut reads: Vec<Read> = if par::few(running.len()) {
        running.into_iter().map(read).collect()
    } else {
        let grain = par::grain(running.len());
        running
            .into_par_iter()
            .with_min_len(grain)
            .map(read)
            .collect()
    };
    reads.sort_unstable_by_key(|read| read.key.flow);
    reads
}

/// Finishes every flow read ahead, in turn, by flow.
fn finish_reads(world: &mut World, reads: Vec<Read>, clock: TickClock, steering: Steering) {
    for read in reads {
        if steering.tracing
            && let Some(mut traced) = world.get_resource_mut::<FlowTrace>()
        {
            traced.steps.extend(read.trace);
        }
        finish(world, read.slot, read.key, read.stopped, clock, steering);
    }
}

/// How far a flow read ahead.
struct Read {
    slot: FlowSlot,
    key: FlowKey,
    stopped: Stopped,
    trace: Vec<TraceStep>,
}

/// Where a flow's reading ahead stopped.
#[derive(Clone, Copy)]
enum Stopped {
    /// At a step that writes the world: a roll of the dice, a dialogue shown, an effect, a
    /// state set.
    Write,
    /// Paused at its node, `Flow::pause` saying for what, and the flow's dialogue box left
    /// open or to close for its subject.
    Paused {
        close: Option<Entity>,
    },
    Ended,
}

/// How the tools steer a run, as of its start.
#[derive(Clone, Copy, Default)]
struct Steering {
    step: bool,
    tracing: bool,
}

impl Steering {
    fn of(world: &World) -> Steering {
        Steering {
            step: world
                .get_resource::<FlowSteering>()
                .is_some_and(|steering| steering.step),
            tracing: world.contains_resource::<FlowTrace>(),
        }
    }
}

/// Runs the flow on from its node through every step that only reads the world, until it
/// pauses, ends, or stands at a step that writes.
fn read_ahead(
    world: &World,
    owners: &HashMap<Owner, Instance>,
    key: FlowKey,
    flow: &mut Flow,
    clock: TickClock,
    steering: Steering,
    trace: &mut Vec<TraceStep>,
) -> Stopped {
    let state = owners.get(&key.owner).map(|instance| &instance.state);
    let mut cx: Option<FlowCx> = None;
    loop {
        if flow
            .frames
            .last()
            .expect("a flow holds a frame while it runs")
            .node
            .is_none()
        {
            if let Some(cx) = cx.take() {
                cx_back(flow, cx);
            }
            if !leave_frame(flow) {
                return Stopped::Ended;
            }
            continue;
        }
        let cx_now = cx.get_or_insert_with(|| cx_of(key, flow, state));
        let top = flow.frames.last().expect("the frame the context came from");
        let node = top.node.expect("checked above");
        let procedure: &Procedure = &top.procedure;
        let mut looping = Looping::Same;
        let outcome = match procedure.step(node) {
            Step::Dialogue { .. } => None,
            Step::Random { weights, outputs } => {
                let steered = world
                    .get_resource::<FlowSteering>()
                    .and_then(|steering| steering.random.get(procedure.name(node)))
                    .copied()
                    .filter(|picked| *picked < outputs.len());
                let picked = steered.unwrap_or_else(|| roll(&mut flow.dice, weights));
                Some(StepOutcome::Next(outputs[picked]))
            }
            Step::Effect {
                look: Some(look),
                outputs,
                ..
            } => look(world, key, cx_now).map(|answer| {
                StepOutcome::Next(match answer {
                    EffectStep::Done => outputs.first().and_then(|(_, to)| *to),
                    EffectStep::Output(name) => output_to(outputs, name),
                    EffectStep::Pause => unreachable!("an effect that has nothing to do goes on"),
                })
            }),
            Step::Effect { look: None, .. } => None,
            Step::Set { values, .. }
                if values
                    .iter()
                    .any(|(slot, _)| matches!(slot, Slot::State(_))) =>
            {
                None
            }
            Step::On { next } => Some(StepOutcome::Next(*next)),
            Step::If { test, yes, no } => {
                Some(StepOutcome::Next(if test(world, cx_now).truthy() {
                    *yes
                } else {
                    *no
                }))
            }
            Step::Repeat {
                times,
                at_most,
                each,
                then,
            } => Some(StepOutcome::Next(match top.loops.last() {
                Some((repeat, remaining)) if *repeat == node => {
                    if *remaining > 0 {
                        looping = Looping::Again;
                        *each
                    } else {
                        looping = Looping::Done;
                        *then
                    }
                }
                _ => {
                    let count = times(world, cx_now).as_int().clamp(0, i64::from(*at_most)) as u32;
                    if count == 0 {
                        *then
                    } else {
                        looping = Looping::Begun(count - 1);
                        *each
                    }
                }
            })),
            Step::Wait {
                seconds, at_most, ..
            } => {
                let seconds = seconds(world, cx_now).as_num().min(f64::from(at_most.0));
                Some(StepOutcome::Pause(if seconds <= 0.0 {
                    Pause::NextTick {
                        tick: clock.tick + 1,
                    }
                } else {
                    Pause::Wait {
                        until: clock.now + Duration::from_secs_f64(seconds),
                    }
                }))
            }
            Step::Set { values, next } => {
                for (slot, to) in values {
                    let value = to(world, cx_now);
                    match slot {
                        Slot::Local(index) => cx_now.locals[*index] = value,
                        Slot::State(_) => {}
                    }
                }
                Some(StepOutcome::Next(*next))
            }
            Step::Run {
                callee,
                entry,
                into,
                next,
            } => Some(StepOutcome::Call {
                callee: callee.clone(),
                entry: *entry,
                into: *into,
                return_to: *next,
            }),
            Step::Return { value } => Some(StepOutcome::Return(
                value.as_ref().map(|value| value(world, cx_now)),
            )),
        };
        let Some(outcome) = outcome else {
            if let Some(cx) = cx.take() {
                cx_back(flow, cx);
            }
            return Stopped::Write;
        };
        if steering.tracing {
            trace.push(TraceStep {
                key,
                procedure: procedure.label.clone(),
                node: procedure.name(node).to_owned(),
            });
        }
        let frame = flow.frames.last_mut().expect("the frame read");
        match looping {
            Looping::Same => {}
            Looping::Begun(count) => frame.loops.push((node, count)),
            Looping::Again => {
                let (_, remaining) = frame.loops.last_mut().expect("the loop");
                *remaining -= 1;
            }
            Looping::Done => {
                frame.loops.pop();
            }
        }
        match outcome {
            StepOutcome::Next(next) => {
                frame.node = next;
                if steering.step && next.is_some() {
                    if let Some(cx) = cx.take() {
                        cx_back(flow, cx);
                    }
                    return pause_at(
                        flow,
                        Pause::Step {
                            tick: clock.tick + 1,
                        },
                    );
                }
            }
            StepOutcome::Pause(pause) => {
                if let Some(cx) = cx.take() {
                    cx_back(flow, cx);
                }
                return pause_at(flow, pause);
            }
            StepOutcome::Call {
                callee,
                entry,
                into,
                return_to,
            } => {
                if let Some(cx) = cx.take() {
                    cx_back(flow, cx);
                }
                let locals = callee.locals.iter().map(|(_, ty)| ty.zero()).collect();
                flow.frames.push(Frame {
                    procedure: callee,
                    node: Some(entry),
                    locals,
                    loops: Vec::new(),
                    into,
                    return_to,
                });
            }
            StepOutcome::Return(value) => {
                if let Some(cx) = cx.take() {
                    cx_back(flow, cx);
                }
                let done = flow.frames.pop().expect("the frame read");
                let Some(caller) = flow.frames.last_mut() else {
                    return Stopped::Ended;
                };
                if let (Some(into), Some(value)) = (done.into, value) {
                    caller.locals[into] = value;
                }
                caller.node = done.return_to;
            }
        }
    }
}

/// What a repeat step read does to its frame's loop.
enum Looping {
    Same,
    Begun(u32),
    Again,
    Done,
}

/// Which of the outputs the dice pick, each as likely as its weight.
fn roll(dice: &mut Rng, weights: &[u32]) -> usize {
    let total: u32 = weights.iter().sum();
    let rolled = dice.rand_range(0..total);
    let mut seen = 0;
    for (index, weight) in weights.iter().enumerate() {
        seen += weight;
        if rolled < seen {
            return index;
        }
    }
    weights.len() - 1
}

/// Where an effect's named output leads.
fn output_to(outputs: &[(&'static str, Option<StepIndex>)], name: &str) -> Option<StepIndex> {
    outputs
        .iter()
        .find(|(output, _)| *output == name)
        .unwrap_or_else(|| panic!("an effect answered {name:?}, not one of its outputs"))
        .1
}

/// Leaves the frame that ran out of nodes: back to the start of its loop, or to its caller.
/// Tells whether the flow goes on.
fn leave_frame(flow: &mut Flow) -> bool {
    let frame = flow
        .frames
        .last_mut()
        .expect("a flow holds a frame while it runs");
    if let Some((repeat, _)) = frame.loops.last() {
        frame.node = Some(*repeat);
        return true;
    }
    let done = flow.frames.pop().expect("the frame left");
    match flow.frames.last_mut() {
        Some(caller) => {
            caller.node = done.return_to;
            true
        }
        None => false,
    }
}

/// Pauses the flow at its node, telling what for, and closes its dialogue box unless the pause
/// keeps it open.
fn pause_at(flow: &mut Flow, pause: Pause) -> Stopped {
    flow.pause = Some(pause);
    flow.running = false;
    let keeps = matches!(pause, Pause::Dialogue | Pause::Effect { keeps_box: true });
    let close = if flow.box_open && !keeps {
        flow.box_open = false;
        flow.subject
    } else {
        None
    };
    Stopped::Paused { close }
}

/// Finishes a flow's run, from where it read ahead to.
fn finish(
    world: &mut World,
    slot: FlowSlot,
    key: FlowKey,
    stopped: Stopped,
    clock: TickClock,
    steering: Steering,
) {
    let mut stopped = stopped;
    loop {
        match stopped {
            Stopped::Ended => return end_flow(world, key),
            Stopped::Paused { close } => {
                if let Some(subject) = close {
                    (Vocabulary::get().dialogue_host().close)(
                        world,
                        key,
                        subject,
                        BoxEnding::Ended,
                    );
                }
                return;
            }
            Stopped::Write => stopped = write_on(world, slot, key, clock, steering),
        }
    }
}

/// Runs the flow on from the step that writes it stands at, reading ahead between writes, until
/// it pauses or ends. The flow is taken out of its slot while its steps write the world, as a
/// step may reach into the instances.
fn write_on(
    world: &mut World,
    slot: FlowSlot,
    key: FlowKey,
    clock: TickClock,
    steering: Steering,
) -> Stopped {
    let mut flow = world
        .resource_mut::<Instances>()
        .flows
        .get_mut(slot)
        .and_then(Option::take)
        .expect("a flow read ahead stands in its slot");
    let stopped = loop {
        if let Some(pause) = write_step(world, key, &mut flow, clock, steering) {
            break pause_at(&mut flow, pause);
        }
        let mut trace = Vec::new();
        let instances = world.resource::<Instances>();
        let stopped = read_ahead(
            world,
            &instances.instances,
            key,
            &mut flow,
            clock,
            steering,
            &mut trace,
        );
        if steering.tracing
            && let Some(mut traced) = world.get_resource_mut::<FlowTrace>()
        {
            traced.steps.extend(trace);
        }
        if !matches!(stopped, Stopped::Write) {
            break stopped;
        }
    };
    if let Some(kept) = world.resource_mut::<Instances>().flows.get_mut(slot) {
        *kept = Some(flow);
    }
    stopped
}

/// Runs the step the flow stands at, which writes the world: the flow goes on from it, or
/// pauses at it for what this says.
fn write_step(
    world: &mut World,
    key: FlowKey,
    flow: &mut Flow,
    clock: TickClock,
    steering: Steering,
) -> Option<Pause> {
    let procedure_id = {
        let procedure = &flow
            .frames
            .last()
            .expect("a flow holds a frame while it runs")
            .procedure;
        (!procedure.state.is_empty()).then_some(procedure.id)
    };
    let held = procedure_id.and_then(|_| world.resource::<Instances>().state(key.owner));
    let mut cx = cx_of(key, flow, held);
    let top = flow.frames.last().expect("the frame the context came from");
    let node = top.node.expect("a step that writes is a node");
    let procedure: &Procedure = &top.procedure;
    if steering.tracing
        && let Some(mut trace) = world.get_resource_mut::<FlowTrace>()
    {
        trace.steps.push(TraceStep {
            key,
            procedure: procedure.label.clone(),
            node: procedure.name(node).to_owned(),
        });
    }
    let mut box_opened = false;
    let outcome: StepOutcome = match procedure.step(node) {
        Step::Set { values, next } => {
            for (slot, to) in values {
                let value = to(world, &cx);
                match slot {
                    Slot::Local(index) => cx.locals[*index] = value,
                    Slot::State(index) => cx.state[*index] = value,
                }
            }
            StepOutcome::Next(*next)
        }
        Step::Dialogue {
            node: dialogue,
            values,
            labels,
            requires,
            choices,
            next,
            ..
        } => match cx.subject {
            None => StepOutcome::Next(None),
            Some(subject) => {
                let mut offers = Vec::new();
                for (index, (choice, label)) in dialogue.choices.iter().zip(labels).enumerate() {
                    let met = match &requires[index] {
                        Some(requires) => requires(world, &cx).truthy(),
                        None => true,
                    };
                    if !met && !choice.reveal.iter().any(|kind| kind.shows_while_locked()) {
                        continue;
                    }
                    let mut leads = Vec::new();
                    let mut to = choices[index];
                    while let Some(at) = to {
                        let Step::Effect {
                            def,
                            outputs,
                            pauses,
                            ..
                        } = procedure.step(at)
                        else {
                            break;
                        };
                        leads.push(def);
                        to = if *pauses {
                            None
                        } else {
                            outputs.first().and_then(|(_, to)| *to)
                        };
                    }
                    offers.push(ChoiceOffer {
                        index,
                        label: &choice.label,
                        label_values: label.iter().map(|eval| eval(world, &cx)).collect(),
                        locked: !met,
                        requires: choice.requires.as_ref(),
                        reveal: choice.reveal.as_slice(),
                        warn: choice.warn.as_deref(),
                        leads,
                    });
                }
                let lines = dialogue
                    .lines
                    .iter()
                    .zip(values)
                    .map(|(line, evals)| {
                        (line, evals.iter().map(|eval| eval(world, &cx)).collect())
                    })
                    .collect();
                (Vocabulary::get().dialogue_host().show)(
                    world,
                    key,
                    subject,
                    DialogueView {
                        script: &procedure.label,
                        camera: procedure.camera,
                        node: procedure.name(node),
                        lines,
                        choices: offers,
                    },
                );
                box_opened = true;
                if dialogue.choices.is_empty() && next.is_some() {
                    StepOutcome::Next(*next)
                } else {
                    StepOutcome::Pause(Pause::Dialogue)
                }
            }
        },
        Step::Effect {
            call,
            outputs,
            keeps_box,
            ..
        } => match call(world, key, &mut cx) {
            EffectStep::Done => StepOutcome::Next(outputs.first().and_then(|(_, to)| *to)),
            EffectStep::Output(name) => StepOutcome::Next(output_to(outputs, name)),
            EffectStep::Pause => {
                // An effect that pauses in the box may be the first to show it, as an item's
                // quest offer is, and the box it shows closes with the flow as a line's does.
                box_opened |= *keeps_box;
                StepOutcome::Pause(Pause::Effect {
                    keeps_box: *keeps_box,
                })
            }
        },
        _ => unreachable!("reading ahead stops only at a step that writes"),
    };
    let state = cx_back(flow, cx);
    if box_opened {
        flow.box_open = true;
    }
    if let Some(procedure) = procedure_id {
        world
            .resource_mut::<Instances>()
            .set_state(key.owner, procedure, state);
    }
    match outcome {
        StepOutcome::Next(next) => {
            flow.frames.last_mut().expect("the frame written").node = next;
            (steering.step && next.is_some()).then_some(Pause::Step {
                tick: clock.tick + 1,
            })
        }
        StepOutcome::Pause(pause) => Some(pause),
        StepOutcome::Call { .. } | StepOutcome::Return(_) => {
            unreachable!("a step that writes neither calls nor returns")
        }
    }
}

/// The context the flow's top frame runs in: its locals and event taken out of the flow, and a
/// copy of its procedure's state from `state`.
fn cx_of(key: FlowKey, flow: &mut Flow, state: Option<&OwnerState>) -> FlowCx {
    let frame = flow
        .frames
        .last_mut()
        .expect("a flow holds a frame while it runs");
    FlowCx {
        owner: key.owner.entity(),
        subject: flow.subject,
        locals: std::mem::take(&mut frame.locals),
        state: state_of(&frame.procedure, state),
        event: std::mem::take(&mut flow.event),
    }
}

/// Gives the locals and event back to the flow's top frame, where `cx_of` took them from; the
/// state is the caller's to keep.
fn cx_back(flow: &mut Flow, cx: FlowCx) -> Vec<Value> {
    let frame = flow
        .frames
        .last_mut()
        .expect("a flow holds a frame while it runs");
    frame.locals = cx.locals;
    flow.event = cx.event;
    cx.state
}

/// A copy of the context the flow's top frame runs in.
fn cx_clone(key: FlowKey, flow: &Flow, state: Option<&OwnerState>) -> FlowCx {
    let frame = flow
        .frames
        .last()
        .expect("a flow holds a frame while it runs");
    FlowCx {
        owner: key.owner.entity(),
        subject: flow.subject,
        locals: frame.locals.clone(),
        state: state_of(&frame.procedure, state),
        event: flow.event.clone(),
    }
}

/// The procedure's state as the owner holds it, or zeroed when it holds none of this shape.
fn state_of(procedure: &Procedure, state: Option<&OwnerState>) -> Vec<Value> {
    if procedure.state.is_empty() {
        return Vec::new();
    }
    state
        .and_then(|state| state.get(&procedure.id))
        .filter(|held| held.len() == procedure.state.len())
        .cloned()
        .unwrap_or_else(|| procedure.state.iter().map(|(_, ty)| ty.zero()).collect())
}

fn apply_ops(world: &mut World) {
    while let Some(op) = world.resource_mut::<Instances>().ops.pop_front() {
        match op {
            Op::Spawn { owner, procedures } => {
                close_instance(world, owner);
                let may_do = MayDo::owned(procedures.iter().map(|procedure| &procedure.may_do));
                let mut instances = world.resource_mut::<Instances>();
                for kind in &may_do.answers {
                    let owners = instances.answering.entry(*kind).or_default();
                    if !owners.contains(&owner) {
                        owners.push(owner);
                    }
                }
                instances.instances.insert(
                    owner,
                    Instance {
                        procedures,
                        may_do,
                        state: OwnerState::new(),
                        flows: Vec::new(),
                    },
                );
            }
            Op::Remove { owner } => close_instance(world, owner),
            Op::Raise {
                owner,
                kind,
                subject,
                event,
                accept,
            } => {
                let starts = {
                    let instances = world.resource::<Instances>();
                    let Some(instance) = instances.instances.get(&owner) else {
                        continue;
                    };
                    let mut starts = Vec::new();
                    for (index, procedure) in instance.procedures.iter().enumerate() {
                        for (entry, def) in procedure.entry(kind) {
                            if !accept(def) {
                                continue;
                            }
                            let live = instance.flows.iter().any(|&slot| {
                                instances
                                    .flows
                                    .get(slot)
                                    .and_then(Option::as_ref)
                                    .is_some_and(|flow| {
                                        flow.procedure == index
                                            && flow.entry == entry
                                            && flow.subject == subject
                                    })
                            });
                            let queued = instances.queue.iter().any(|queued| match queued {
                                Queued::Start {
                                    owner: o,
                                    procedure: p,
                                    entry: e,
                                    subject: s,
                                    ..
                                } => *o == owner && *p == index && *e == entry && *s == subject,
                                Queued::Resume { .. } => false,
                            });
                            if live || queued {
                                continue;
                            }
                            starts.push((index, entry));
                        }
                    }
                    starts
                };
                for (procedure, entry) in starts {
                    world
                        .resource_mut::<Instances>()
                        .queue
                        .push_back(Queued::Start {
                            owner,
                            procedure,
                            entry,
                            subject,
                            event: event.clone(),
                        });
                }
            }
            Op::Answer { key, answer } => {
                world
                    .resource_mut::<Instances>()
                    .queue
                    .push_back(Queued::Resume { key, answer });
            }
            Op::BoxClosed { key } => {
                if let Some(flow) = world.resource_mut::<Instances>().flow_mut(key) {
                    flow.box_open = false;
                }
            }
        }
    }
}

fn close_instance(world: &mut World, owner: Owner) {
    let removed = {
        let mut instances = world.resource_mut::<Instances>();
        instances
            .queue
            .retain(|queued| queued_owner(queued) != owner);
        let removed = instances.instances.remove(&owner);
        if let Some(instance) = &removed {
            for kind in &instance.may_do.answers {
                if let Some(owners) = instances.answering.get_mut(kind) {
                    owners.retain(|known| *known != owner);
                }
            }
        }
        removed
    };
    let closing: Vec<(FlowKey, Entity)> = removed.map_or_else(Vec::new, |instance| {
        let mut instances = world.resource_mut::<Instances>();
        let mut closing = Vec::new();
        for &slot in &instance.flows {
            let Some(flow) = instances.flows.remove(slot).flatten() else {
                continue;
            };
            instances.by_id.remove(&flow.id);
            if flow.box_open
                && let Some(subject) = flow.subject
            {
                let key = FlowKey {
                    owner,
                    flow: flow.id,
                };
                closing.push((key, subject));
            }
        }
        closing
    });
    for (key, subject) in closing {
        (Vocabulary::get().dialogue_host().close)(world, key, subject, BoxEnding::Gone);
    }
}

fn queued_owner(queued: &Queued) -> Owner {
    match queued {
        Queued::Start { owner, .. } => *owner,
        Queued::Resume { key, .. } => key.owner,
    }
}

/// Resumes a paused flow in its slot with the answer it waited for, and tells whether it goes
/// on: an answer that isn't the one it waits for leaves it as it is.
fn resume(world: &mut World, key: FlowKey, answer: Answer) -> bool {
    let instances = world.resource::<Instances>();
    let Some(flow) = instances.flow(key) else {
        return false;
    };
    let Some(pause) = flow.pause else {
        return false;
    };
    let Some(top) = flow.frames.last() else {
        return false;
    };
    let Some(node) = top.node else {
        return false;
    };
    let procedure = Arc::clone(&top.procedure);
    let mut leaves_box = false;
    let next = match (procedure.step(node), pause, &answer) {
        (
            Step::Wait { next, .. },
            Pause::Wait { .. } | Pause::NextTick { .. },
            Answer::Output(_),
        ) => Some(*next),
        (
            Step::Dialogue {
                requires, choices, ..
            },
            Pause::Dialogue,
            Answer::Pick(choice),
        ) => match choices.get(*choice).copied() {
            Some(to) => match requires.get(*choice) {
                Some(Some(requires)) => {
                    let cx = cx_clone(key, flow, instances.state(key.owner));
                    requires(world, &cx).truthy().then_some(to)
                }
                _ => Some(to),
            },
            None => None,
        },
        (Step::Dialogue { left, .. }, Pause::Dialogue, Answer::Left) => {
            leaves_box = true;
            Some(*left)
        }
        (Step::Effect { outputs, .. }, Pause::Effect { .. }, Answer::Output(name)) => outputs
            .iter()
            .find(|(output, _)| output == name)
            .map(|(_, to)| *to),
        (_, Pause::Step { .. }, Answer::Output(_)) => Some(Some(node)),
        _ => None,
    };
    let Some(next) = next else {
        return false;
    };
    let mut instances = world.resource_mut::<Instances>();
    let flow = instances.flow_mut(key).expect("the flow resumed");
    if leaves_box {
        flow.box_open = false;
    }
    flow.frames.last_mut().expect("the frame resumed").node = next;
    flow.pause = None;
    flow.running = true;
    true
}

/// Every node a flow stepped through while this resource was in the world, for tools that watch a
/// script run.
#[derive(Resource, Default)]
pub struct FlowTrace {
    pub steps: Vec<TraceStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep {
    pub key: FlowKey,
    pub procedure: String,
    pub node: String,
}

/// How a tool steers a run while this resource is in the world: the output each named random node
/// takes instead of rolling, and whether every node pauses until the next tick so a run can be
/// stepped.
#[derive(Resource, Default)]
pub struct FlowSteering {
    pub random: HashMap<String, usize>,
    pub step: bool,
}

/// A raiser may hand fewer values than the entry's event declares; the rest read as zero.
fn padded(mut event: Vec<Value>, root: &Procedure, entry: StepIndex) -> Vec<Value> {
    let fields = root
        .entries
        .iter()
        .find(|(at, _)| *at == entry)
        .map_or(&[][..], |(_, raised)| raised.fields());
    for (_, ty) in fields.iter().skip(event.len()) {
        event.push(ty.zero());
    }
    event
}

enum StepOutcome {
    Next(Option<StepIndex>),
    Pause(Pause),
    Call {
        callee: Arc<Procedure>,
        entry: StepIndex,
        into: Option<usize>,
        return_to: Option<StepIndex>,
    },
    Return(Option<Value>),
}

/// Drops a flow that ran to its end, closing its dialogue box if it left one open.
fn end_flow(world: &mut World, key: FlowKey) {
    let close = {
        let mut instances = world.resource_mut::<Instances>();
        let close = instances
            .flow(key)
            .filter(|flow| flow.box_open)
            .and_then(|flow| flow.subject);
        instances.drop_flow(key);
        close
    };
    if let Some(subject) = close {
        (Vocabulary::get().dialogue_host().close)(world, key, subject, BoxEnding::Ended);
    }
}

fn peek_dialogues(
    world: &World,
    instance: &Instance,
    procedure: &Procedure,
    entry: StepIndex,
    cx: FlowCx,
    found: &mut Vec<(String, String)>,
) {
    let mut todo = vec![(entry, cx)];
    let mut visited = HashSet::new();
    while let Some((node, mut cx)) = todo.pop() {
        if !visited.insert(node) {
            continue;
        }
        match procedure.step(node) {
            Step::Dialogue { .. } => {
                found.push((procedure.label.clone(), procedure.name(node).to_owned()))
            }
            Step::Return { .. } => {}
            Step::If { test, yes, no } => {
                let next = if test(world, &cx).truthy() { *yes } else { *no };
                todo.extend(next.map(|next| (next, cx)));
            }
            Step::Set { values, next } => {
                for (slot, to) in values {
                    let value = to(world, &cx);
                    match slot {
                        Slot::Local(index) => cx.locals[*index] = value,
                        Slot::State(index) => cx.state[*index] = value,
                    }
                }
                todo.extend(next.map(|next| (next, cx)));
            }
            Step::Run {
                callee,
                entry,
                next,
                ..
            } => {
                let callee_cx = fresh_cx(instance, callee, cx.owner, cx.subject);
                peek_dialogues(world, instance, callee, *entry, callee_cx, found);
                todo.extend(next.map(|next| (next, cx)));
            }
            _ => todo.extend(
                procedure
                    .successors(node)
                    .into_iter()
                    .map(|next| (next, cx.clone())),
            ),
        }
    }
}

fn fresh_cx(
    instance: &Instance,
    procedure: &Procedure,
    owner: Option<Entity>,
    subject: Option<Entity>,
) -> FlowCx {
    FlowCx {
        owner,
        subject,
        locals: procedure.locals.iter().map(|(_, ty)| ty.zero()).collect(),
        state: instance
            .state
            .get(&procedure.id)
            .cloned()
            .unwrap_or_else(|| procedure.state.iter().map(|(_, ty)| ty.zero()).collect()),
        event: Vec::new(),
    }
}
