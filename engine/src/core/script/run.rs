use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_time::Time;
use slotmap::{SlotMap, new_key_type};
use smallvec::{SmallVec, smallvec};

use super::compile::{EffectStep, Procedure, ProcedureId, Slot, Step, StepIndex};
use super::dialogue::{ChoiceOffer, DialogueView};
use super::may_do::MayDo;
use super::node::MAX_LIVE_FLOWS;
use super::value::Value;
use super::{EventDef, Name, RevealDef, Vocabulary};
use crate::core::math::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner<V: Vocabulary> {
    Entity(V::OwnerKind, Entity),
    Row(V::OwnerKind, u32),
}

impl<V: Vocabulary> Owner<V> {
    pub fn kind(self) -> V::OwnerKind {
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
pub struct FlowKey<V: Vocabulary> {
    pub owner: Owner<V>,
    pub flow: FlowId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlowId(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pause {
    Wait { until: f64 },
    NextTick { tick: u64 },
    Step { tick: u64 },
    Dialogue,
    Effect { keeps_box: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Output(Name),
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

type Accept<V> = Box<dyn Fn(&<V as Vocabulary>::Event) -> bool + Send + Sync>;

enum Op<V: Vocabulary> {
    Spawn {
        owner: Owner<V>,
        procedures: Vec<Arc<Procedure<V>>>,
    },
    Remove {
        owner: Owner<V>,
    },
    Raise {
        owner: Owner<V>,
        kind: <V::Event as EventDef<V>>::Kind,
        subject: Option<Entity>,
        event: Vec<Value>,
        accept: Accept<V>,
    },
    Answer {
        key: FlowKey<V>,
        answer: Answer,
    },
    BoxClosed {
        key: FlowKey<V>,
    },
}

#[derive(Resource)]
pub struct Instances<V: Vocabulary> {
    instances: HashMap<Owner<V>, Instance<V>>,
    answering: HashMap<<V::Event as EventDef<V>>::Kind, Vec<Owner<V>>>,
    ops: VecDeque<Op<V>>,
    queue: VecDeque<Queued<V>>,
    flows: SlotMap<FlowSlot, Option<Flow<V>>>,
    by_id: HashMap<FlowId, FlowSlot>,
    next_flow: FlowId,
    tick: u64,
}

new_key_type! {
    struct FlowSlot;
}

impl<V: Vocabulary> Default for Instances<V> {
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

enum Queued<V: Vocabulary> {
    Start {
        owner: Owner<V>,
        procedure: usize,
        entry: StepIndex,
        subject: Option<Entity>,
        event: Vec<Value>,
    },
    Resume {
        key: FlowKey<V>,
        answer: Answer,
    },
}

struct Instance<V: Vocabulary> {
    procedures: Vec<Arc<Procedure<V>>>,
    may_do: MayDo<V>,
    state: HashMap<ProcedureId, Vec<Value>>,
    flows: Vec<FlowSlot>,
}

struct Flow<V: Vocabulary> {
    owner: Owner<V>,
    id: FlowId,
    procedure: usize,
    entry: StepIndex,
    subject: Option<Entity>,
    event: Vec<Value>,
    frames: SmallVec<[Frame<V>; 1]>,
    pause: Option<Pause>,
    box_open: bool,
}

struct Frame<V: Vocabulary> {
    procedure: Arc<Procedure<V>>,
    node: Option<StepIndex>,
    locals: Locals,
    loops: Vec<(StepIndex, u32)>,
    into: Option<usize>,
    return_to: Option<StepIndex>,
}

impl<V: Vocabulary> Instances<V> {
    pub fn spawn_owner(&mut self, owner: Owner<V>, procedures: Vec<Arc<Procedure<V>>>) {
        self.ops.push_back(Op::Spawn { owner, procedures });
    }

    pub fn remove_owner(&mut self, owner: Owner<V>) {
        self.ops.push_back(Op::Remove { owner });
    }

    pub fn raise(
        &mut self,
        owner: Owner<V>,
        kind: <V::Event as EventDef<V>>::Kind,
        subject: Option<Entity>,
        event: Vec<Value>,
        accept: impl Fn(&V::Event) -> bool + Send + Sync + 'static,
    ) {
        self.ops.push_back(Op::Raise {
            owner,
            kind,
            subject,
            event,
            accept: Box::new(accept),
        });
    }

    pub fn answer(&mut self, key: FlowKey<V>, answer: Answer) {
        self.ops.push_back(Op::Answer { key, answer });
    }

    pub fn box_closed(&mut self, key: FlowKey<V>) {
        self.ops.push_back(Op::BoxClosed { key });
    }

    pub fn owners(&self) -> impl Iterator<Item = Owner<V>> + '_ {
        self.instances.keys().copied()
    }

    pub fn may_do(&self, owner: Owner<V>) -> Option<&MayDo<V>> {
        self.instances.get(&owner).map(|instance| &instance.may_do)
    }

    pub fn owners_answering(&self, kind: <V::Event as EventDef<V>>::Kind) -> &[Owner<V>] {
        self.answering.get(&kind).map_or(&[], Vec::as_slice)
    }

    pub fn answers(&self, owner: Owner<V>, kind: <V::Event as EventDef<V>>::Kind) -> bool {
        self.instances
            .get(&owner)
            .is_some_and(|instance| instance.may_do.answers(kind))
    }

    pub fn entries(
        &self,
        owner: Owner<V>,
        kind: <V::Event as EventDef<V>>::Kind,
    ) -> impl Iterator<Item = &V::Event> {
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
        owner: Owner<V>,
        kind: <V::Event as EventDef<V>>::Kind,
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

    pub fn pause_of(&self, key: FlowKey<V>) -> Option<Pause> {
        self.flow(key).and_then(|flow| flow.pause)
    }

    pub fn live_flows(&self, owner: Owner<V>) -> usize {
        self.instances
            .get(&owner)
            .map_or(0, |instance| instance.flows.len())
    }

    fn flow(&self, key: FlowKey<V>) -> Option<&Flow<V>> {
        let slot = *self.by_id.get(&key.flow)?;
        self.flows.get(slot)?.as_ref()
    }

    fn flow_mut(&mut self, key: FlowKey<V>) -> Option<&mut Flow<V>> {
        let slot = *self.by_id.get(&key.flow)?;
        self.flows.get_mut(slot)?.as_mut()
    }

    fn take_state(&mut self, owner: Owner<V>, procedure: &Procedure<V>) -> Vec<Value> {
        let zeroed = || procedure.state.iter().map(|(_, ty)| ty.zero()).collect();
        if procedure.state.is_empty() {
            return Vec::new();
        }
        let Some(instance) = self.instances.get_mut(&owner) else {
            return zeroed();
        };
        let state = instance.state.entry(procedure.id).or_default();
        if state.len() != procedure.state.len() {
            *state = zeroed();
        }
        std::mem::take(state)
    }

    fn put_state(&mut self, owner: Owner<V>, procedure: &Procedure<V>, state: Vec<Value>) {
        if procedure.state.is_empty() {
            return;
        }
        if let Some(instance) = self.instances.get_mut(&owner) {
            instance.state.insert(procedure.id, state);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceState {
    Open,
    Locked,
    Gone,
}

pub fn choice_state<V: Vocabulary>(
    world: &mut World,
    key: FlowKey<V>,
    choice: usize,
) -> ChoiceState {
    match choice_open::<V>(world, key, choice) {
        Some(true) => ChoiceState::Open,
        Some(false) => ChoiceState::Locked,
        None => ChoiceState::Gone,
    }
}

fn choice_open<V: Vocabulary>(world: &mut World, key: FlowKey<V>, choice: usize) -> Option<bool> {
    let mut flow = world.resource_mut::<Instances<V>>().take_flow(key)?;
    let open = open_choice_of(world, key, &mut flow, choice);
    put_flow(world, key, flow);
    open
}

fn open_choice_of<V: Vocabulary>(
    world: &mut World,
    key: FlowKey<V>,
    flow: &mut Flow<V>,
    choice: usize,
) -> Option<bool> {
    if flow.pause != Some(Pause::Dialogue) {
        return None;
    }
    let node = flow.frames.last()?.node?;
    let procedure = flow.frames.last()?.procedure.clone();
    if !matches!(procedure.step(node), Step::Dialogue { .. }) {
        return None;
    }
    let mut frame = flow.frames.pop()?;
    let cx = FlowCx {
        owner: key.owner.entity(),
        subject: flow.subject,
        locals: std::mem::take(&mut frame.locals),
        state: world
            .resource_mut::<Instances<V>>()
            .take_state(key.owner, &procedure),
        event: std::mem::take(&mut flow.event),
    };
    let open = match procedure.step(node) {
        Step::Dialogue { requires, .. } => requires.get(choice).map(|requires| {
            requires
                .as_ref()
                .is_none_or(|requires| requires(world, &cx).truthy())
        }),
        _ => None,
    };
    give_back(world, &mut frame, flow, cx);
    flow.frames.push(frame);
    open
}

pub fn tick<V: Vocabulary>(world: &mut World) {
    if !world.contains_resource::<Instances<V>>() {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    let tick = {
        let mut instances = world.resource_mut::<Instances<V>>();
        instances.tick += 1;
        let tick = instances.tick;
        let mut due: Vec<FlowKey<V>> = instances
            .flows
            .values()
            .flatten()
            .filter(|flow| match flow.pause {
                Some(Pause::Wait { until }) => until <= now,
                Some(Pause::NextTick { tick: at }) | Some(Pause::Step { tick: at }) => at <= tick,
                _ => false,
            })
            .map(|flow| FlowKey {
                owner: flow.owner,
                flow: flow.id,
            })
            .collect();
        due.sort_by_key(|key| key.flow);
        for key in due {
            instances.queue.push_back(Queued::Resume {
                key,
                answer: Answer::Output("next"),
            });
        }
        tick
    };
    let clock = TickClock { tick, now };
    let mut rounds = 0;
    loop {
        let picked = {
            let mut instances = world.resource_mut::<Instances<V>>();
            let pending_ops = !instances.ops.is_empty();
            if pending_ops {
                apply_ops::<V>(world);
                continue;
            }
            match instances.queue.pop_front() {
                None => break,
                Some(Queued::Start {
                    owner,
                    procedure,
                    entry,
                    subject,
                    event,
                }) => {
                    if instances.is_full(owner) {
                        Some(Picked::Dropped(owner))
                    } else {
                        instances
                            .start_flow(owner, procedure, entry, subject, event)
                            .map(|(key, flow)| Picked::Start(key, flow))
                    }
                }
                Some(Queued::Resume { key, answer }) => instances
                    .take_flow(key)
                    .map(|flow| Picked::Resume(key, flow, answer)),
            }
        };
        match picked {
            Some(Picked::Dropped(owner)) => V::log_dropped_flow(world, &owner, "32 live flows"),
            Some(Picked::Start(key, flow)) => run_flow::<V>(world, key, flow, clock),
            Some(Picked::Resume(key, flow, answer)) => {
                if let Some(flow) = resume::<V>(world, key, flow, answer) {
                    run_flow::<V>(world, key, flow, clock);
                }
            }
            None => {}
        }
        rounds += 1;
        if rounds > 100_000 {
            break;
        }
    }
}

#[derive(Clone, Copy)]
struct TickClock {
    tick: u64,
    now: f64,
}

enum Picked<V: Vocabulary> {
    Dropped(Owner<V>),
    Start(FlowKey<V>, Flow<V>),
    Resume(FlowKey<V>, Flow<V>, Answer),
}

impl<V: Vocabulary> Instances<V> {
    fn start_flow(
        &mut self,
        owner: Owner<V>,
        procedure: usize,
        entry: StepIndex,
        subject: Option<Entity>,
        event: Vec<Value>,
    ) -> Option<(FlowKey<V>, Flow<V>)> {
        let instance = self.instances.get_mut(&owner)?;
        let id = self.next_flow;
        self.next_flow = FlowId(id.0 + 1);
        let slot = self.flows.insert(None);
        self.by_id.insert(id, slot);
        instance.flows.push(slot);
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
        };
        Some((FlowKey { owner, flow: id }, flow))
    }

    fn is_full(&self, owner: Owner<V>) -> bool {
        self.instances
            .get(&owner)
            .is_some_and(|instance| instance.flows.len() >= MAX_LIVE_FLOWS)
    }

    fn take_flow(&mut self, key: FlowKey<V>) -> Option<Flow<V>> {
        let slot = *self.by_id.get(&key.flow)?;
        self.flows.get_mut(slot)?.take()
    }

    fn drop_flow(&mut self, key: FlowKey<V>) {
        let Some(slot) = self.by_id.remove(&key.flow) else {
            return;
        };
        self.flows.remove(slot);
        if let Some(instance) = self.instances.get_mut(&key.owner) {
            instance.flows.retain(|kept| *kept != slot);
        }
    }
}

fn apply_ops<V: Vocabulary>(world: &mut World) {
    while let Some(op) = world.resource_mut::<Instances<V>>().ops.pop_front() {
        match op {
            Op::Spawn { owner, procedures } => {
                close_instance(world, owner);
                let mut may_do = MayDo::default();
                for procedure in &procedures {
                    may_do.absorb(&procedure.may_do);
                    may_do
                        .answers
                        .extend(procedure.may_do.answers.iter().copied());
                }
                let mut instances = world.resource_mut::<Instances<V>>();
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
                        state: HashMap::new(),
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
                    let instances = world.resource::<Instances<V>>();
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
                        .resource_mut::<Instances<V>>()
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
                    .resource_mut::<Instances<V>>()
                    .queue
                    .push_back(Queued::Resume { key, answer });
            }
            Op::BoxClosed { key } => {
                if let Some(flow) = world.resource_mut::<Instances<V>>().flow_mut(key) {
                    flow.box_open = false;
                }
            }
        }
    }
}

fn close_instance<V: Vocabulary>(world: &mut World, owner: Owner<V>) {
    let removed = {
        let mut instances = world.resource_mut::<Instances<V>>();
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
    let closing: Vec<(FlowKey<V>, Entity)> = removed.map_or_else(Vec::new, |instance| {
        let mut instances = world.resource_mut::<Instances<V>>();
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
        V::close_dialogue(world, key, subject, BoxEnding::Gone);
    }
}

fn queued_owner<V: Vocabulary>(queued: &Queued<V>) -> Owner<V> {
    match queued {
        Queued::Start { owner, .. } => *owner,
        Queued::Resume { key, .. } => key.owner,
    }
}

fn resume<V: Vocabulary>(
    world: &mut World,
    key: FlowKey<V>,
    mut flow: Flow<V>,
    answer: Answer,
) -> Option<Flow<V>> {
    let Some(pause) = flow.pause else {
        return keep(world, key, flow);
    };
    let mut frame = flow.frames.pop().expect("a paused flow has a frame");
    let Some(node) = frame.node else {
        flow.frames.push(frame);
        return keep(world, key, flow);
    };
    let next = match (frame.procedure.step(node), pause, &answer) {
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
                    let cx = FlowCx {
                        owner: key.owner.entity(),
                        subject: flow.subject,
                        locals: std::mem::take(&mut frame.locals),
                        state: world
                            .resource_mut::<Instances<V>>()
                            .take_state(key.owner, &frame.procedure),
                        event: std::mem::take(&mut flow.event),
                    };
                    let met = requires(world, &cx).truthy();
                    give_back(world, &mut frame, &mut flow, cx);
                    met.then_some(to)
                }
                _ => Some(to),
            },
            None => None,
        },
        (Step::Dialogue { left, .. }, Pause::Dialogue, Answer::Left) => {
            flow.box_open = false;
            Some(*left)
        }
        (Step::Effect { outputs, .. }, Pause::Effect { .. }, Answer::Output(name)) => outputs
            .iter()
            .find(|(output, _)| output == name)
            .map(|(_, to)| *to),
        (_, Pause::Step { .. }, Answer::Output(_)) => Some(frame.node),
        _ => None,
    };
    let Some(next) = next else {
        flow.frames.push(frame);
        return keep(world, key, flow);
    };
    frame.node = next;
    flow.frames.push(frame);
    flow.pause = None;
    Some(flow)
}

fn keep<V: Vocabulary>(world: &mut World, key: FlowKey<V>, flow: Flow<V>) -> Option<Flow<V>> {
    put_flow(world, key, flow);
    None
}

fn put_flow<V: Vocabulary>(world: &mut World, key: FlowKey<V>, flow: Flow<V>) {
    let mut instances = world.resource_mut::<Instances<V>>();
    let Some(slot) = instances.by_id.get(&key.flow).copied() else {
        return;
    };
    if let Some(kept) = instances.flows.get_mut(slot) {
        *kept = Some(flow);
    }
}

/// Every node a flow stepped through while this resource was in the world, for tools that watch a
/// script run.
#[derive(Resource)]
pub struct FlowTrace<V: Vocabulary> {
    pub steps: Vec<TraceStep<V>>,
}

impl<V: Vocabulary> Default for FlowTrace<V> {
    fn default() -> FlowTrace<V> {
        FlowTrace { steps: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep<V: Vocabulary> {
    pub key: FlowKey<V>,
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
fn padded<V: Vocabulary>(
    mut event: Vec<Value>,
    root: &Procedure<V>,
    entry: StepIndex,
) -> Vec<Value> {
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

fn run_flow<V: Vocabulary>(world: &mut World, key: FlowKey<V>, flow: Flow<V>, clock: TickClock) {
    let mut trace = world.remove_resource::<FlowTrace<V>>();
    run_flow_traced(world, key, flow, clock, &mut trace);
    if let Some(trace) = trace {
        world.insert_resource(trace);
    }
}

fn run_flow_traced<V: Vocabulary>(
    world: &mut World,
    key: FlowKey<V>,
    mut flow: Flow<V>,
    clock: TickClock,
    trace: &mut Option<FlowTrace<V>>,
) {
    loop {
        let mut frame = flow.frames.pop().expect("a frame");
        let Some(node) = frame.node else {
            if let Some((repeat, _)) = frame.loops.last() {
                frame.node = Some(*repeat);
                flow.frames.push(frame);
                continue;
            }
            if let Some(caller) = flow.frames.last_mut() {
                caller.node = frame.return_to;
                continue;
            }
            end_flow::<V>(world, key, flow, BoxEnding::Ended);
            return;
        };
        let procedure = Arc::clone(&frame.procedure);
        let mut cx = FlowCx {
            owner: key.owner.entity(),
            subject: flow.subject,
            locals: std::mem::take(&mut frame.locals),
            state: world
                .resource_mut::<Instances<V>>()
                .take_state(key.owner, &procedure),
            event: std::mem::take(&mut flow.event),
        };
        if let Some(trace) = trace.as_mut() {
            trace.steps.push(TraceStep {
                key,
                procedure: procedure.label.clone(),
                node: procedure.name(node).to_owned(),
            });
        }
        let outcome;
        match procedure.step(node) {
            Step::On { next } => outcome = StepOutcome::Next(*next),
            Step::If { test, yes, no } => {
                outcome = StepOutcome::Next(if test(world, &cx).truthy() { *yes } else { *no });
            }
            Step::Random { weights, outputs } => {
                let steered = world
                    .get_resource::<FlowSteering>()
                    .and_then(|steering| steering.random.get(procedure.name(node)))
                    .copied()
                    .filter(|picked| *picked < outputs.len());
                let picked = steered.unwrap_or_else(|| {
                    let total: u32 = weights.iter().sum();
                    let roll = world.resource_mut::<Rng>().rand_range(0..total);
                    let mut seen = 0;
                    let mut picked = outputs.len() - 1;
                    for (index, weight) in weights.iter().enumerate() {
                        seen += weight;
                        if roll < seen {
                            picked = index;
                            break;
                        }
                    }
                    picked
                });
                outcome = StepOutcome::Next(outputs[picked]);
            }
            Step::Repeat {
                times,
                at_most,
                each,
                then,
            } => {
                let looping = frame
                    .loops
                    .last()
                    .is_some_and(|(repeat, _)| *repeat == node);
                if looping {
                    let (_, remaining) = frame.loops.last_mut().expect("the loop");
                    if *remaining > 0 {
                        *remaining -= 1;
                        outcome = StepOutcome::Next(*each);
                    } else {
                        frame.loops.pop();
                        outcome = StepOutcome::Next(*then);
                    }
                } else {
                    let count = times(world, &cx).as_int().clamp(0, i64::from(*at_most)) as u32;
                    if count == 0 {
                        outcome = StepOutcome::Next(*then);
                    } else {
                        frame.loops.push((node, count - 1));
                        outcome = StepOutcome::Next(*each);
                    }
                }
            }
            Step::Wait {
                seconds, at_most, ..
            } => {
                let seconds = seconds(world, &cx).as_num().min(f64::from(*at_most));
                let pause = if seconds <= 0.0 {
                    Pause::NextTick {
                        tick: clock.tick + 1,
                    }
                } else {
                    Pause::Wait {
                        until: clock.now + seconds,
                    }
                };
                outcome = StepOutcome::Pause(pause);
            }
            Step::Set { values, next } => {
                for (slot, to) in values {
                    let value = to(world, &cx);
                    match slot {
                        Slot::Local(index) => cx.locals[*index] = value,
                        Slot::State(index) => cx.state[*index] = value,
                    }
                }
                outcome = StepOutcome::Next(*next);
            }
            Step::Run {
                callee,
                entry,
                into,
                next,
            } => {
                outcome = StepOutcome::Call {
                    callee: callee.clone(),
                    entry: *entry,
                    into: *into,
                    return_to: *next,
                };
            }
            Step::Return { value } => {
                outcome = StepOutcome::Return(value.as_ref().map(|value| value(world, &cx)));
            }
            Step::Dialogue {
                node: dialogue,
                values,
                labels,
                requires,
                choices,
                next,
                ..
            } => {
                let Some(subject) = cx.subject else {
                    give_back(world, &mut frame, &mut flow, cx);
                    frame.node = None;
                    flow.frames.push(frame);
                    continue;
                };
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
                V::show_dialogue(
                    world,
                    key,
                    subject,
                    DialogueView {
                        script: &procedure.label,
                        node: procedure.name(node),
                        lines,
                        choices: offers,
                    },
                );
                flow.box_open = true;
                outcome = if dialogue.choices.is_empty() && next.is_some() {
                    StepOutcome::Next(*next)
                } else {
                    StepOutcome::Pause(Pause::Dialogue)
                };
            }
            Step::Effect {
                call,
                outputs,
                keeps_box,
                ..
            } => {
                outcome = match call(world, key, &mut cx) {
                    EffectStep::Done => StepOutcome::Next(outputs.first().and_then(|(_, to)| *to)),
                    EffectStep::Output(name) => StepOutcome::Next(
                        outputs
                            .iter()
                            .find(|(output, _)| *output == name)
                            .unwrap_or_else(|| {
                                panic!("an effect answered {name:?}, not one of its outputs")
                            })
                            .1,
                    ),
                    EffectStep::Pause => StepOutcome::Pause(Pause::Effect {
                        keeps_box: *keeps_box,
                    }),
                };
            }
        }
        give_back(world, &mut frame, &mut flow, cx);
        let stepping = world
            .get_resource::<FlowSteering>()
            .is_some_and(|steering| steering.step);
        if stepping && let StepOutcome::Next(Some(next)) = outcome {
            frame.node = Some(next);
            flow.frames.push(frame);
            flow.pause = Some(Pause::Step {
                tick: clock.tick + 1,
            });
            put_flow(world, key, flow);
            return;
        }
        match outcome {
            StepOutcome::Next(next) => {
                frame.node = next;
                flow.frames.push(frame);
            }
            StepOutcome::Pause(pause) => {
                flow.frames.push(frame);
                flow.pause = Some(pause);
                let keeps = matches!(pause, Pause::Dialogue | Pause::Effect { keeps_box: true });
                let close = if flow.box_open && !keeps {
                    flow.box_open = false;
                    flow.subject
                } else {
                    None
                };
                put_flow(world, key, flow);
                if let Some(subject) = close {
                    V::close_dialogue(world, key, subject, BoxEnding::Ended);
                }
                return;
            }
            StepOutcome::Call {
                callee,
                entry,
                into,
                return_to,
            } => {
                let locals = callee.locals.iter().map(|(_, ty)| ty.zero()).collect();
                flow.frames.push(frame);
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
                let Some(caller) = flow.frames.last_mut() else {
                    end_flow::<V>(world, key, flow, BoxEnding::Ended);
                    return;
                };
                if let (Some(into), Some(value)) = (frame.into, value) {
                    caller.locals[into] = value;
                }
                caller.node = frame.return_to;
            }
        }
    }
}

fn give_back<V: Vocabulary>(
    world: &mut World,
    frame: &mut Frame<V>,
    flow: &mut Flow<V>,
    cx: FlowCx,
) {
    frame.locals = cx.locals;
    flow.event = cx.event;
    world
        .resource_mut::<Instances<V>>()
        .put_state(flow.owner, &frame.procedure, cx.state);
}

enum StepOutcome<V: Vocabulary> {
    Next(Option<StepIndex>),
    Pause(Pause),
    Call {
        callee: Arc<Procedure<V>>,
        entry: StepIndex,
        into: Option<usize>,
        return_to: Option<StepIndex>,
    },
    Return(Option<Value>),
}

fn end_flow<V: Vocabulary>(world: &mut World, key: FlowKey<V>, flow: Flow<V>, ending: BoxEnding) {
    world.resource_mut::<Instances<V>>().drop_flow(key);
    if flow.box_open
        && let Some(subject) = flow.subject
    {
        V::close_dialogue(world, key, subject, ending);
    }
}

fn peek_dialogues<V: Vocabulary>(
    world: &World,
    instance: &Instance<V>,
    procedure: &Procedure<V>,
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

fn fresh_cx<V: Vocabulary>(
    instance: &Instance<V>,
    procedure: &Procedure<V>,
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
