use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy_ecs::prelude::*;

use super::dialogue::{DialogueNode, MAX_CHOICES, MAX_LINES, check_reveals_unique};
use super::expr::{self, Expr};
use super::may_do::MayDo;
use super::node::{
    MAX_CALL_DEPTH, MAX_NODES, MAX_REPEAT, MAX_SLOTS, MAX_STEPS, MAX_WAIT, Node, Target,
};
use super::run::{FlowCx, FlowKey};
use super::value::{Value, ValueType};
use super::{
    Arrow, EffectDef, EventDef, Fill, InlineScript, LineDef, Name, OwnerKindDef, QueryDef,
    ScriptError, ScriptFault, ScriptId, ScriptModule, TextDef, VarValue, Variable, VariableType,
    Vocabulary,
};
use crate::core::content::{Content, ContentId, ContentRow};

pub type Eval = Box<dyn Fn(&World, &FlowCx) -> Value + Send + Sync>;
pub type EffectCall<V> =
    Box<dyn Fn(&mut World, FlowKey<V>, &mut FlowCx) -> EffectStep + Send + Sync>;

pub enum EffectStep {
    /// Along the effect's first output: its way on when all went as planned.
    Done,
    Output(Name),
    Pause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OwnerRow<V: Vocabulary> {
    pub kind: V::OwnerKind,
    pub row: u32,
    pub object: Option<u32>,
}

impl<V: Vocabulary> OwnerRow<V> {
    pub fn id<Row: ContentRow>(&self) -> ContentId<Row> {
        ContentId::new(self.row)
    }
}

#[derive(Resource, Clone, Default)]
pub struct Scripts<V: Vocabulary>(pub Arc<ByOwner<V>>);

type ByOwner<V> = HashMap<OwnerRow<V>, Vec<Arc<Procedure<V>>>>;

impl<V: Vocabulary> Scripts<V> {
    pub fn of(&self, owner: OwnerRow<V>) -> &[Arc<Procedure<V>>] {
        self.0.get(&owner).map_or(&[], Vec::as_slice)
    }

    pub fn may_do(&self, owner: OwnerRow<V>) -> MayDo<V> {
        let mut all = MayDo::default();
        for procedure in self.of(owner) {
            all.absorb(&procedure.may_do);
            all.answers.extend(procedure.may_do.answers.iter().copied());
        }
        all
    }

    pub fn owners(&self) -> impl Iterator<Item = (&OwnerRow<V>, &Vec<Arc<Procedure<V>>>)> {
        self.0.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StepIndex(pub(super) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcedureId(u64);

pub struct Procedure<V: Vocabulary> {
    pub id: ProcedureId,
    pub owner: V::OwnerKind,
    pub label: String,
    pub names: Vec<String>,
    pub steps: Vec<Step<V>>,
    pub entries: Vec<(StepIndex, V::Event)>,
    pub state: Vec<(String, ValueType)>,
    pub locals: Vec<(String, ValueType)>,
    pub may_do: MayDo<V>,
    pub returns: Vec<ValueType>,
    pub needs_subject: bool,
}

pub enum Step<V: Vocabulary> {
    On {
        next: Option<StepIndex>,
    },
    If {
        test: Eval,
        yes: Option<StepIndex>,
        no: Option<StepIndex>,
    },
    Random {
        weights: Vec<u32>,
        outputs: Vec<Option<StepIndex>>,
    },
    Repeat {
        times: Eval,
        at_most: u32,
        each: Option<StepIndex>,
        then: Option<StepIndex>,
    },
    Wait {
        seconds: Eval,
        at_most: f32,
        next: Option<StepIndex>,
    },
    Set {
        values: Vec<(Slot, Eval)>,
        next: Option<StepIndex>,
    },
    Run {
        callee: Arc<Procedure<V>>,
        entry: StepIndex,
        into: Option<usize>,
        next: Option<StepIndex>,
    },
    Return {
        value: Option<Eval>,
    },
    Dialogue {
        node: DialogueNode<V>,
        values: Vec<Vec<Eval>>,
        labels: Vec<Vec<Eval>>,
        requires: Vec<Option<Eval>>,
        choices: Vec<Option<StepIndex>>,
        next: Option<StepIndex>,
        left: Option<StepIndex>,
    },
    Effect {
        def: V::Effect,
        call: EffectCall<V>,
        outputs: Vec<(Name, Option<StepIndex>)>,
        pauses: bool,
        keeps_box: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Local(usize),
    State(usize),
}

impl<V: Vocabulary> Procedure<V> {
    pub fn entry(&self, kind: <V::Event as EventDef<V>>::Kind) -> Vec<(StepIndex, &V::Event)> {
        self.entries
            .iter()
            .filter(|(_, event)| event.kind() == kind)
            .map(|(node, event)| (*node, event))
            .collect()
    }

    pub fn answers(&self, kind: <V::Event as EventDef<V>>::Kind) -> bool {
        self.entries.iter().any(|(_, event)| event.kind() == kind)
    }

    pub fn run_entry(&self) -> Option<StepIndex> {
        self.entries
            .iter()
            .find(|(_, event)| event.is_run())
            .map(|(node, _)| *node)
    }

    pub fn step(&self, at: StepIndex) -> &Step<V> {
        &self.steps[at.0]
    }

    pub fn successors(&self, at: StepIndex) -> Vec<StepIndex> {
        step_successors(self.step(at))
    }

    pub fn name(&self, at: StepIndex) -> &str {
        &self.names[at.0]
    }
}

impl<V: Vocabulary> Step<V> {
    pub fn pauses(&self) -> bool {
        match self {
            Step::Wait { .. } | Step::Dialogue { .. } => true,
            Step::Effect { pauses, .. } => *pauses,
            _ => false,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Call<'c, V: Vocabulary> {
    Shared {
        script: ScriptId<V>,
        values: &'c BTreeMap<String, VarValue<V>>,
    },
    Inline(&'c InlineScript<V>),
}

impl<'c, V: Vocabulary> Call<'c, V> {
    pub fn of(module: &'c ScriptModule<V>) -> Call<'c, V> {
        match module {
            ScriptModule::Shared { script, values } => Call::Shared {
                script: *script,
                values,
            },
            ScriptModule::Inline(inline) => Call::Inline(inline),
        }
    }
}

pub struct Binding<'c, V: Vocabulary> {
    pub owner: V::OwnerKind,
    pub row: Option<OwnerRow<V>>,
    pub label: String,
    pub call: Call<'c, V>,
}

#[derive(Clone)]
pub enum Bound<'c, V: Vocabulary> {
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Text(&'c str),
    Name(&'c str),
    Place(V::Place),
    Row(V::Row),
    Audio(V::Audio),
    Condition(&'c Expr<V>),
}

pub type ModuleErrors = Vec<(usize, ScriptError)>;
type ValuesOf<'c, V> = Option<&'c BTreeMap<String, VarValue<V>>>;
type DeclaredOf<'c, V> = Option<&'c BTreeMap<String, Variable<V>>>;

pub fn compile_owner<'c, V: Vocabulary>(
    content: &'c Content,
    owner: V::OwnerKind,
    label: &str,
    modules: &[&'c ScriptModule<V>],
) -> Result<Vec<Arc<Procedure<V>>>, ModuleErrors> {
    compile_modules(content, owner, None, label, modules)
}

/// The scripts of one row, which may then read which row they belong to.
pub fn compile_row<'c, V: Vocabulary>(
    content: &'c Content,
    row: OwnerRow<V>,
    label: &str,
    modules: &[&'c ScriptModule<V>],
) -> Result<Vec<Arc<Procedure<V>>>, ModuleErrors> {
    compile_modules(content, row.kind, Some(row), label, modules)
}

fn compile_modules<'c, V: Vocabulary>(
    content: &'c Content,
    owner: V::OwnerKind,
    row: Option<OwnerRow<V>>,
    label: &str,
    modules: &[&'c ScriptModule<V>],
) -> Result<Vec<Arc<Procedure<V>>>, ModuleErrors> {
    let mut procedures = Vec::new();
    let mut errors = Vec::new();
    let mut answered: HashMap<<V::Event as EventDef<V>>::Kind, Vec<String>> = HashMap::new();
    for (index, module) in modules.iter().enumerate() {
        let procedure = match compile_binding(
            content,
            &Binding {
                owner,
                row,
                label: label.to_owned(),
                call: Call::of(module),
            },
        ) {
            Ok(procedure) => procedure,
            Err(error) => {
                errors.push((index, error));
                continue;
            }
        };
        for (node, event) in &procedure.entries {
            let twice = answered.entry(event.kind()).or_default();
            if let Some(first) = twice.first() {
                errors.push((
                    index,
                    ScriptError {
                        rule: "answered twice",
                        owner: label.to_owned(),
                        node: Some(procedure.name(*node).to_owned()),
                        expected: format!(
                            "one answer to {:?} per owner; {first} answers it too",
                            event.kind()
                        ),
                        script: None,
                    },
                ));
            }
            twice.push(procedure.name(*node).to_owned());
        }
        procedures.push(procedure);
    }
    if errors.is_empty() {
        Ok(procedures)
    } else {
        Err(errors)
    }
}

pub fn compile_binding<'c, V: Vocabulary>(
    content: &'c Content,
    binding: &Binding<'c, V>,
) -> Result<Arc<Procedure<V>>, ScriptError> {
    compile_source(content, binding, &[])
}

fn slots(declared: &BTreeMap<String, ValueType>) -> Vec<(&str, ValueType)> {
    declared
        .iter()
        .map(|(name, ty)| (name.as_str(), *ty))
        .collect()
}

fn resolve_value<'c, V: Vocabulary>(
    content: &Content,
    name: &str,
    declared: &'c Variable<V>,
    value: &'c VarValue<V>,
) -> Result<Bound<'c, V>, ScriptFault> {
    let mismatch = || {
        ScriptFault::new(
            "wrong type",
            format!("{name} is a {:?}, given {}", declared.ty, value.kind()),
        )
    };
    Ok(match (declared.ty, value) {
        (VariableType::Integer, VarValue::Integer(value)) => Bound::Integer(*value),
        (VariableType::Number, VarValue::Integer(value)) => Bound::Number(*value as f64),
        (VariableType::Number, VarValue::Number(value)) => Bound::Number(*value),
        (VariableType::Boolean, VarValue::Boolean(value)) => Bound::Boolean(*value),
        (VariableType::Text, VarValue::Str(value)) => Bound::Text(value),
        (VariableType::Name, VarValue::Str(value)) => Bound::Name(value),
        (VariableType::Place, VarValue::Place(place)) => Bound::Place(place.clone()),
        (VariableType::Row, VarValue::Str(key)) => {
            let table = declared.table.as_deref().ok_or_else(|| {
                ScriptFault::new(
                    "wrong type",
                    format!("{name} is a row variable that names no table"),
                )
            })?;
            Bound::Row(V::row_value(content, table, key).ok_or_else(|| {
                ScriptFault::new(
                    "unknown row",
                    format!("{key:?} is not the key of a {table} row"),
                )
            })?)
        }
        (VariableType::Audio, VarValue::Str(key)) => {
            Bound::Audio(V::audio_value(content, key).ok_or_else(|| {
                ScriptFault::new("unknown row", format!("{key:?} is not the key of a sound"))
            })?)
        }
        (VariableType::Condition, VarValue::Condition(expr)) => Bound::Condition(expr),
        (VariableType::Condition, VarValue::Boolean(value)) => Bound::Boolean(*value),
        _ => return Err(mismatch()),
    })
}

fn compile_source<'c, V: Vocabulary>(
    content: &'c Content,
    binding: &Binding<'c, V>,
    stack: &[ScriptId<V>],
) -> Result<Arc<Procedure<V>>, ScriptError> {
    let shared = match binding.call {
        Call::Shared { script, .. } => Some(script.key(content).to_owned()),
        Call::Inline(_) => None,
    };
    let fail = |rule: &'static str, node: Option<&str>, expected: String| ScriptError {
        rule,
        owner: binding.label.clone(),
        node: node.map(str::to_owned),
        expected,
        script: shared.clone(),
    };
    let at_call = |rule: &'static str, expected: String| ScriptError {
        rule,
        owner: binding.label.clone(),
        node: None,
        expected,
        script: None,
    };
    let (source, values, declared): (Source<'c, V>, ValuesOf<'c, V>, DeclaredOf<'c, V>) =
        match binding.call {
            Call::Shared { script, values } => {
                let def = script.get(content);
                let called = !stack.is_empty();
                if called {
                    if def.owner != V::OwnerKind::ANY && def.owner != binding.owner {
                        return Err(at_call(
                            "owners of another kind",
                            format!(
                                "script/{} is {}'s, run by {}",
                                script.key(content),
                                def.owner.name(),
                                binding.owner.name()
                            ),
                        ));
                    }
                } else if def.owner != binding.owner {
                    return Err(at_call(
                        "owners of another kind",
                        format!("script/{} is {}'s", script.key(content), def.owner.name()),
                    ));
                }
                (
                    Source {
                        state: slots(&def.state),
                        locals: slots(&def.locals),
                        nodes: &def.nodes,
                        label: format!("script/{}", script.key(content)),
                    },
                    Some(values),
                    Some(&def.variables),
                )
            }
            Call::Inline(inline) => (
                Source {
                    state: slots(&inline.state),
                    locals: slots(&inline.locals),
                    nodes: &inline.nodes,
                    label: binding.label.clone(),
                },
                None,
                None,
            ),
        };
    if source.nodes.len() > MAX_NODES {
        return Err(fail(
            "out of range",
            None,
            format!("at most {MAX_NODES} nodes, found {}", source.nodes.len()),
        ));
    }
    for (what, count) in [
        ("variables", declared.map_or(0, BTreeMap::len)),
        ("locals", source.locals.len()),
        ("state", source.state.len()),
    ] {
        if count > MAX_SLOTS {
            return Err(fail(
                "out of range",
                None,
                format!("at most {MAX_SLOTS} {what}, found {count}"),
            ));
        }
    }
    if !source.state.is_empty() && !binding.owner.keeps_state() {
        return Err(fail(
            "state kept nowhere",
            None,
            format!(
                "{} scripts keep no state; found {}",
                binding.owner.name(),
                source.state[0].0
            ),
        ));
    }
    let mut variables: HashMap<&'c str, (VariableType, Option<Bound<'c, V>>)> = HashMap::new();
    if let Some(declared) = declared {
        for (name, variable) in declared {
            let default = match &variable.default {
                Some(default) => Some(
                    resolve_value(content, name, variable, default)
                        .map_err(|fault| fail(fault.rule, None, fault.expected))?,
                ),
                None => None,
            };
            variables.insert(name.as_str(), (variable.ty, default));
        }
    }
    for (name, value) in values.into_iter().flatten() {
        let Some(variable) = declared.and_then(|declared| declared.get(name)) else {
            return Err(at_call(
                "unknown variable",
                format!(
                    "{name} is not a variable of {}{}",
                    source.label,
                    nearest(
                        name,
                        declared
                            .into_iter()
                            .flat_map(|declared| declared.keys().map(String::as_str))
                    )
                ),
            ));
        };
        let bound = resolve_value(content, name, variable, value)
            .map_err(|fault| at_call(fault.rule, fault.expected))?;
        variables.get_mut(name.as_str()).expect("declared above").1 = Some(bound);
    }
    for (name, (_, value)) in &variables {
        if value.is_none() {
            return Err(at_call(
                "missing value",
                format!("{name} has no default and no value"),
            ));
        }
    }
    let names: Vec<&'c str> = source.nodes.keys().map(String::as_str).collect();
    let mut seen = HashSet::new();
    for name in &names {
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(fail(
                "duplicate name",
                Some(name),
                "unique node names".to_owned(),
            ));
        }
    }
    let index_of = |name: &str| names.iter().position(|known| *known == name).map(StepIndex);
    let arrow = |arrow: &'c Arrow, at: &'c str| -> Result<Option<StepIndex>, ScriptError> {
        match arrow.as_deref() {
            None => Ok(None),
            Some(name) => index_of(name).map(Some).ok_or_else(|| {
                fail(
                    "unknown node",
                    Some(at),
                    format!("{name:?}{}", nearest(name, names.iter().copied())),
                )
            }),
        }
    };
    let mut cx = Compiler {
        content,
        owner: binding.owner,
        row: binding.row,
        label: source.label.clone(),
        variables,
        state: source.state.clone(),
        locals: source.locals.clone(),
        node: None,
        used_vars: HashSet::new(),
        read_slots: HashSet::new(),
        stack: stack.to_vec(),
        subject_read: false,
        event_fields: &[],
    };
    let mut steps = Vec::with_capacity(source.nodes.len());
    let mut entries = Vec::new();
    let event_fields = event_fields_by_node(source.nodes);
    let mut may_do = MayDo::default();
    let mut needs_subject = vec![false; names.len()];
    let mut returns = Vec::new();
    for (index, (name, node)) in source.nodes.iter().enumerate() {
        let name: &'c str = name.as_str();
        cx.node = Some(name);
        cx.event_fields = event_fields.get(name).copied().unwrap_or(&[]);
        let node_fail = |rule: &'static str, expected: String| fail(rule, Some(name), expected);
        let step = match node {
            Node::On { event, next } => {
                let function =
                    event.is_run() && (binding.owner == V::OwnerKind::ANY || !cx.stack.is_empty());
                if !event.owners().contains(&binding.owner) && !function {
                    return Err(node_fail(
                        "event not raised here",
                        format!(
                            "{:?} is never raised for {}",
                            event.kind(),
                            binding.owner.name()
                        ),
                    ));
                }
                if cx.stack.is_empty() && event.is_run() && binding.owner != V::OwnerKind::ANY {
                    return Err(node_fail(
                        "event not raised here",
                        "run starts only a script another runs".to_owned(),
                    ));
                }
                entries.push((StepIndex(index), event.clone()));
                may_do.answers.push(event.kind());
                Step::On {
                    next: arrow(next, name)?,
                }
            }
            Node::If { test, yes, no } => {
                let (test, uses) = cx
                    .expr_tracking(test, ValueType::Boolean)
                    .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                needs_subject[index] |= uses;
                Step::If {
                    test,
                    yes: arrow(yes, name)?,
                    no: arrow(no, name)?,
                }
            }
            Node::Random { weights, outputs } => {
                if weights.len() < 2 || weights.len() > 8 || weights.len() != outputs.len() {
                    return Err(node_fail(
                        "out of range",
                        "2 to 8 weights, one output each".to_owned(),
                    ));
                }
                if weights.iter().any(|weight| !(1..=1_000).contains(weight)) {
                    return Err(node_fail("out of range", "weights 1 to 1,000".to_owned()));
                }
                Step::Random {
                    weights: weights.clone(),
                    outputs: outputs
                        .iter()
                        .map(|output| arrow(output, name))
                        .collect::<Result<_, _>>()?,
                }
            }
            Node::Repeat {
                times,
                at_most,
                each,
                then,
            } => {
                if !(1..=MAX_REPEAT).contains(at_most) {
                    return Err(node_fail(
                        "out of range",
                        format!("at_most 1 to {MAX_REPEAT}"),
                    ));
                }
                let (times, uses) = cx
                    .expr_tracking(times, ValueType::Integer)
                    .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                needs_subject[index] |= uses;
                Step::Repeat {
                    times,
                    at_most: *at_most,
                    each: arrow(each, name)?,
                    then: arrow(then, name)?,
                }
            }
            Node::Wait {
                seconds,
                at_most,
                next,
            } => {
                if !(*at_most > 0.0 && *at_most <= MAX_WAIT) {
                    return Err(node_fail(
                        "out of range",
                        format!("at_most up to {MAX_WAIT} seconds"),
                    ));
                }
                let (seconds, uses) = cx
                    .expr_tracking(seconds, ValueType::Number)
                    .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                needs_subject[index] |= uses;
                may_do.waits = true;
                Step::Wait {
                    seconds,
                    at_most: *at_most,
                    next: arrow(next, name)?,
                }
            }
            Node::Set { values, next } => {
                let mut compiled = Vec::new();
                for assignment in values.iter() {
                    let (slot, ty) = match &assignment.target {
                        Target::Local(local) => {
                            cx.local(local).map(|(index, ty)| (Slot::Local(index), ty))
                        }
                        Target::State(state) => {
                            cx.state(state).map(|(index, ty)| (Slot::State(index), ty))
                        }
                    }
                    .ok_or_else(|| {
                        node_fail("unknown variable", format!("{:?}", assignment.target))
                    })?;
                    let (to, uses) = cx
                        .expr_tracking(&assignment.to, ty)
                        .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                    needs_subject[index] |= uses;
                    compiled.push((slot, to));
                }
                Step::Set {
                    values: compiled,
                    next: arrow(next, name)?,
                }
            }
            Node::Run {
                script,
                values,
                into,
                next,
            } => {
                if cx.stack.contains(script) {
                    return Err(node_fail(
                        "runs itself",
                        format!("script/{} reaches itself", script.key(content)),
                    ));
                }
                if cx.stack.len() + 1 > MAX_CALL_DEPTH {
                    return Err(node_fail(
                        "runs itself",
                        format!("calls nest past {MAX_CALL_DEPTH}"),
                    ));
                }
                let mut deeper = cx.stack.clone();
                deeper.push(*script);
                let callee = compile_source(
                    content,
                    &Binding {
                        owner: binding.owner,
                        row: binding.row,
                        label: format!("{} › script/{}", source.label, script.key(content)),
                        call: Call::Shared {
                            script: *script,
                            values,
                        },
                    },
                    &deeper,
                )?;
                let entry = callee.run_entry().ok_or_else(|| {
                    node_fail(
                        "event not raised here",
                        format!("script/{} has no run node", script.key(content)),
                    )
                })?;
                let into = match into {
                    Some(local) => {
                        let (slot, ty) = cx.local(local).ok_or_else(|| {
                            node_fail("unknown variable", format!("local {local}"))
                        })?;
                        for returned in callee.returns.iter().copied() {
                            if !ty.accepts(returned) {
                                return Err(node_fail(
                                    "wrong type",
                                    format!(
                                        "{local} is {} but script/{} returns {}",
                                        ty.name(),
                                        script.key(content),
                                        returned.name()
                                    ),
                                ));
                            }
                        }
                        Some(slot)
                    }
                    None => None,
                };
                may_do.absorb(&callee.may_do);
                may_do.calls.push(script.key(content).to_owned());
                needs_subject[index] |= callee.needs_subject;
                Step::Run {
                    callee,
                    entry,
                    into,
                    next: arrow(next, name)?,
                }
            }
            Node::Return { value } => {
                if cx.stack.is_empty() {
                    return Err(node_fail(
                        "event not raised here",
                        "return belongs in a script another runs".to_owned(),
                    ));
                }
                let value = match value {
                    Some(value) => {
                        let (eval, ty, uses) = cx
                            .typed_tracking(value)
                            .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                        needs_subject[index] |= uses;
                        returns.push(ty);
                        Some(eval)
                    }
                    None => None,
                };
                Step::Return { value }
            }
            Node::Dialogue(dialogue) => {
                if dialogue.lines.is_empty() || dialogue.lines.len() > MAX_LINES {
                    return Err(node_fail("out of range", format!("1 to {MAX_LINES} lines")));
                }
                if dialogue.choices.len() > MAX_CHOICES {
                    return Err(node_fail(
                        "out of range",
                        format!("at most {MAX_CHOICES} choices"),
                    ));
                }
                if dialogue.next.is_some() && !dialogue.choices.is_empty() {
                    return Err(node_fail(
                        "wrong type",
                        "next only without choices".to_owned(),
                    ));
                }
                needs_subject[index] = true;
                may_do.dialogue.push(name.to_owned());
                let mut values = Vec::new();
                for line in &dialogue.lines {
                    let mut evals = Vec::new();
                    for value in line.values() {
                        evals.push(
                            cx.typed(value)
                                .map_err(|fault| node_fail(fault.rule, fault.expected))?
                                .0,
                        );
                    }
                    values.push(evals);
                }
                let mut labels = Vec::new();
                let mut requires = Vec::new();
                let mut choices = Vec::new();
                for choice in &dialogue.choices {
                    let mut evals = Vec::new();
                    for value in choice.label.values() {
                        evals.push(
                            cx.typed(value)
                                .map_err(|fault| node_fail(fault.rule, fault.expected))?
                                .0,
                        );
                    }
                    labels.push(evals);
                    check_reveals_unique(&choice.reveal)
                        .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                    if choice
                        .warn
                        .as_ref()
                        .is_some_and(|warn| warn.chars().count() > 60)
                    {
                        return Err(node_fail(
                            "out of range",
                            "warn at most 60 characters".to_owned(),
                        ));
                    }
                    requires.push(match &choice.requires {
                        Some(expr) => Some(
                            cx.expr(expr, ValueType::Boolean)
                                .map_err(|fault| node_fail(fault.rule, fault.expected))?,
                        ),
                        None => None,
                    });
                    choices.push(arrow(&choice.to, name)?);
                }
                Step::Dialogue {
                    node: dialogue.clone(),
                    values,
                    labels,
                    requires,
                    choices,
                    next: arrow(&dialogue.next, name)?,
                    left: arrow(&dialogue.left, name)?,
                }
            }
            Node::Effect(effect) => {
                if !effect.owners().contains(&binding.owner) && binding.owner != V::OwnerKind::ANY {
                    return Err(node_fail(
                        "not for this owner",
                        format!("not for {} owners", binding.owner.name()),
                    ));
                }
                cx.subject_read = false;
                let call = effect
                    .compile(&mut cx)
                    .map_err(|fault| node_fail(fault.rule, fault.expected))?;
                needs_subject[index] |= effect.needs_subject() || cx.subject_read;
                may_do.reaches.extend(effect.reach(&cx));
                let outputs = effect
                    .outputs()
                    .into_iter()
                    .map(|(output, to)| match to {
                        None => Ok((output, None)),
                        Some(to) => index_of(to).map(|to| (output, Some(to))).ok_or_else(|| {
                            node_fail(
                                "unknown node",
                                format!("{to:?}{}", nearest(to, names.iter().copied())),
                            )
                        }),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Step::Effect {
                    def: effect.clone(),
                    call,
                    outputs,
                    pauses: effect.pauses(),
                    keeps_box: effect.keeps_box(),
                }
            }
        };
        steps.push(step);
    }
    cx.node = None;
    if entries.is_empty() {
        return Err(fail("unreachable node", None, "no on node".to_owned()));
    }
    for name in cx.variables.keys() {
        if !cx.used_vars.contains(name) {
            return Err(fail(
                "unused variable",
                None,
                format!("{name} is never used"),
            ));
        }
    }
    for (index, (name, _)) in source.locals.iter().enumerate() {
        if !cx.read_slots.contains(&Slot::Local(index)) {
            return Err(fail(
                "unused local",
                None,
                format!("local {name} is never read"),
            ));
        }
    }
    for (index, (name, _)) in source.state.iter().enumerate() {
        if !cx.read_slots.contains(&Slot::State(index)) {
            return Err(fail(
                "unused local",
                None,
                format!("state {name} is never read"),
            ));
        }
    }
    let successors: Vec<Vec<StepIndex>> = steps.iter().map(step_successors).collect();
    let mut reachable = vec![false; names.len()];
    let mut subject_free = vec![false; names.len()];
    for (entry, event) in &entries {
        let mut todo = vec![*entry];
        while let Some(StepIndex(node)) = todo.pop() {
            if reachable[node] && (event.has_subject() || subject_free[node]) {
                continue;
            }
            reachable[node] = true;
            if !event.has_subject() {
                subject_free[node] = true;
            }
            todo.extend(successors[node].iter().copied());
        }
    }
    for (index, name) in names.iter().enumerate() {
        if !reachable[index] {
            return Err(fail(
                "unreachable node",
                Some(name),
                "no on node leads here".to_owned(),
            ));
        }
        if subject_free[index] && needs_subject[index] {
            return Err(fail(
                "no subject here",
                Some(name),
                "reached from an event without a subject".to_owned(),
            ));
        }
    }
    bound_steps(&steps, &names, &successors, &entries, &fail)?;
    let any_needs_subject = needs_subject.iter().any(|needs| *needs);
    Ok(Arc::new(Procedure {
        id: ProcedureId(NEXT_PROCEDURE.fetch_add(1, Ordering::Relaxed)),
        owner: binding.owner,
        label: source.label,
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        steps,
        entries,
        state: owned_slots(&source.state),
        locals: owned_slots(&source.locals),
        may_do,
        returns,
        needs_subject: any_needs_subject,
    }))
}

static NEXT_PROCEDURE: AtomicU64 = AtomicU64::new(1);

struct Source<'c, V: Vocabulary> {
    state: Vec<(&'c str, ValueType)>,
    locals: Vec<(&'c str, ValueType)>,
    nodes: &'c BTreeMap<String, Node<V>>,
    label: String,
}

fn owned_slots(slots: &[(&str, ValueType)]) -> Vec<(String, ValueType)> {
    slots
        .iter()
        .map(|(name, ty)| ((*name).to_owned(), *ty))
        .collect()
}

/// The event fields a node can read: those of the event every entry reaching it raises. A node
/// reached by entries that raise different fields reads none, since a flow carries only the fields
/// of the entry that started it.
fn event_fields_by_node<V: Vocabulary>(
    nodes: &BTreeMap<String, Node<V>>,
) -> HashMap<&str, &'static [(Name, ValueType)]> {
    let mut fields: HashMap<&str, Option<&'static [(Name, ValueType)]>> = HashMap::new();
    for (entry, node) in nodes {
        let Node::On { event, .. } = node else {
            continue;
        };
        let raised = event.fields();
        let mut seen = HashSet::new();
        let mut todo = vec![entry.as_str()];
        while let Some(name) = todo.pop() {
            if !seen.insert(name) {
                continue;
            }
            let slot = fields.entry(name).or_insert(Some(raised));
            if slot.is_some_and(|known| known != raised) {
                *slot = None;
            }
            if let Some(node) = nodes.get(name) {
                todo.extend(node_arrows(node));
            }
        }
    }
    fields
        .into_iter()
        .filter_map(|(name, fields)| fields.map(|fields| (name, fields)))
        .collect()
}

fn node_arrows<V: Vocabulary>(node: &Node<V>) -> Vec<&str> {
    let arrows: Vec<&Arrow> = match node {
        Node::On { next, .. }
        | Node::Wait { next, .. }
        | Node::Set { next, .. }
        | Node::Run { next, .. } => vec![next],
        Node::If { yes, no, .. } => vec![yes, no],
        Node::Random { outputs, .. } => outputs.iter().collect(),
        Node::Repeat { each, then, .. } => vec![each, then],
        Node::Return { .. } => Vec::new(),
        Node::Dialogue(dialogue) => dialogue
            .choices
            .iter()
            .map(|choice| &choice.to)
            .chain([&dialogue.next, &dialogue.left])
            .collect(),
        Node::Effect(effect) => {
            return effect
                .outputs()
                .into_iter()
                .filter_map(|(_, to)| to)
                .collect();
        }
    };
    arrows.into_iter().filter_map(Option::as_deref).collect()
}

fn step_successors<V: Vocabulary>(step: &Step<V>) -> Vec<StepIndex> {
    let arrows: Vec<Option<StepIndex>> = match step {
        Step::On { next } | Step::Wait { next, .. } | Step::Set { next, .. } => vec![*next],
        Step::If { yes, no, .. } => vec![*yes, *no],
        Step::Random { outputs, .. } => outputs.clone(),
        Step::Repeat { each, then, .. } => vec![*each, *then],
        Step::Run { next, .. } => vec![*next],
        Step::Return { .. } => vec![],
        Step::Dialogue {
            choices,
            next,
            left,
            ..
        } => {
            let mut all = choices.clone();
            all.push(*next);
            all.push(*left);
            all
        }
        Step::Effect { outputs, .. } => outputs.iter().map(|(_, to)| *to).collect(),
    };
    arrows.into_iter().flatten().collect()
}

fn bound_steps<V: Vocabulary>(
    steps: &[Step<V>],
    names: &[&str],
    successors: &[Vec<StepIndex>],
    entries: &[(StepIndex, V::Event)],
    fail: &impl Fn(&'static str, Option<&str>, String) -> ScriptError,
) -> Result<(), ScriptError> {
    let mut walk = Walk {
        steps,
        names,
        successors,
        path: Vec::new(),
        memo: HashMap::new(),
        fail,
    };
    let mut sources: Vec<StepIndex> = entries.iter().map(|(entry, _)| *entry).collect();
    sources.extend(
        (0..steps.len())
            .filter(|node| steps[*node].pauses())
            .map(StepIndex),
    );
    for source in sources {
        let starts: Vec<StepIndex> = if steps[source.0].pauses() {
            successors[source.0].clone()
        } else {
            vec![source]
        };
        for start in starts {
            walk.longest(start)?;
        }
    }
    Ok(())
}

struct Walk<'a, V: Vocabulary, F> {
    steps: &'a [Step<V>],
    names: &'a [&'a str],
    successors: &'a [Vec<StepIndex>],
    path: Vec<StepIndex>,
    memo: HashMap<StepIndex, usize>,
    fail: &'a F,
}

impl<V: Vocabulary, F: Fn(&'static str, Option<&str>, String) -> ScriptError> Walk<'_, V, F> {
    fn longest(&mut self, node: StepIndex) -> Result<usize, ScriptError> {
        if self.steps[node.0].pauses() {
            return Ok(1);
        }
        if let Some(known) = self.memo.get(&node) {
            return Ok(*known);
        }
        if self.path.contains(&node) {
            let cycle: Vec<&str> = self
                .path
                .iter()
                .skip_while(|seen| **seen != node)
                .map(|seen| self.names[seen.0])
                .chain([self.names[node.0]])
                .collect();
            return Err((self.fail)(
                "loop without a wait",
                Some(self.names[node.0]),
                cycle.join(" → "),
            ));
        }
        self.path.push(node);
        let cost = match &self.steps[node.0] {
            Step::Repeat {
                at_most,
                each,
                then,
                ..
            } => {
                let body = match each {
                    Some(each) => self.longest(*each)?,
                    None => 0,
                };
                let after = match then {
                    Some(then) => self.longest(*then)?,
                    None => 0,
                };
                (*at_most as usize) * (1 + body) + 1 + after
            }
            _ => {
                let mut most = 0;
                for next in &self.successors[node.0] {
                    most = most.max(self.longest(*next)?);
                }
                1 + most
            }
        };
        self.path.pop();
        if cost > MAX_STEPS {
            return Err((self.fail)(
                "too many steps",
                Some(self.names[node.0]),
                format!("{cost} steps between pauses, at most {MAX_STEPS}"),
            ));
        }
        self.memo.insert(node, cost);
        Ok(cost)
    }
}

pub fn nearest<'a>(wanted: &str, known: impl Iterator<Item = &'a str>) -> String {
    let known: Vec<String> = known.map(str::to_owned).collect();
    crate::core::content::id::nearest(&known, wanted)
        .map(|near| format!(" · nearest {near:?}"))
        .unwrap_or_default()
}

pub struct Compiler<'c, V: Vocabulary> {
    pub content: &'c Content,
    owner: V::OwnerKind,
    row: Option<OwnerRow<V>>,
    label: String,
    variables: HashMap<&'c str, (VariableType, Option<Bound<'c, V>>)>,
    state: Vec<(&'c str, ValueType)>,
    locals: Vec<(&'c str, ValueType)>,
    node: Option<&'c str>,
    used_vars: HashSet<&'c str>,
    read_slots: HashSet<Slot>,
    stack: Vec<ScriptId<V>>,
    subject_read: bool,
    event_fields: &'static [(Name, ValueType)],
}

type Arithmetic = (fn(i64, i64) -> i64, fn(f64, f64) -> f64);

impl<'c, V: Vocabulary> Compiler<'c, V> {
    pub fn standalone(content: &'c Content) -> Compiler<'c, V> {
        Compiler {
            content,
            owner: V::OwnerKind::ANY,
            row: None,
            label: "condition".to_owned(),
            variables: HashMap::new(),
            state: Vec::new(),
            locals: Vec::new(),
            node: None,
            used_vars: HashSet::new(),
            read_slots: HashSet::new(),
            stack: Vec::new(),
            subject_read: false,
            event_fields: &[],
        }
    }

    pub fn owner(&self) -> V::OwnerKind {
        self.owner
    }

    pub fn content(&self) -> &'c Content {
        self.content
    }

    /// The row whose script this is, when it is a row's own.
    pub fn owner_row(&self) -> Option<OwnerRow<V>> {
        self.row
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn node(&self) -> Option<&'c str> {
        self.node
    }

    pub fn expr(&mut self, expr: &'c Expr<V>, want: ValueType) -> Result<Eval, ScriptFault> {
        let (eval, ty) = self.typed(expr)?;
        if !want.accepts(ty) {
            return Err(ScriptFault::new(
                "wrong type",
                format!("{} is {}, expected {}", expr.kind(), ty.name(), want.name()),
            ));
        }
        Ok(if ty == want {
            eval
        } else {
            Box::new(move |world, cx| eval(world, cx).widened(want))
        })
    }

    pub fn opt_expr(
        &mut self,
        expr: Option<&'c Expr<V>>,
        want: ValueType,
    ) -> Result<Option<Eval>, ScriptFault> {
        expr.map(|expr| self.expr(expr, want)).transpose()
    }

    pub fn actor(&mut self, expr: Option<&'c Expr<V>>) -> Result<Eval, ScriptFault> {
        match expr {
            Some(expr) => self.expr(expr, ValueType::Actor),
            None => {
                self.subject_read = true;
                Ok(Box::new(|_, cx: &FlowCx| Value::Actor(cx.subject)))
            }
        }
    }

    /// One actor or a set of them; the subject alone when nothing is named.
    pub fn actors(&mut self, expr: Option<&'c Expr<V>>) -> Result<Eval, ScriptFault> {
        match expr {
            Some(expr) => self.expr(expr, ValueType::Actors),
            None => {
                self.subject_read = true;
                Ok(Box::new(|_, cx: &FlowCx| {
                    Value::Actors(cx.subject.into_iter().collect())
                }))
            }
        }
    }

    pub fn fill<T: Clone>(
        &mut self,
        fill: &Fill<T>,
        ty: VariableType,
        extract: impl Fn(&Bound<V>) -> Option<T>,
    ) -> Result<T, ScriptFault> {
        match fill {
            Fill::Const(value) => Ok(value.clone()),
            Fill::Var { var } => {
                let value = self.var(var)?;
                extract(value)
                    .ok_or_else(|| ScriptFault::new("wrong type", format!("{var} is not a {ty:?}")))
            }
        }
    }

    pub fn peek<T: Clone>(
        &self,
        fill: &Fill<T>,
        extract: impl Fn(&Bound<V>) -> Option<T>,
    ) -> Option<T> {
        match fill {
            Fill::Const(value) => Some(value.clone()),
            Fill::Var { var } => self
                .variables
                .get(var.as_str())
                .and_then(|(_, value)| value.as_ref().and_then(extract)),
        }
    }

    pub fn peek_place(&self, fill: &Fill<V::Place>) -> Option<V::Place> {
        self.peek(fill, |value| match value {
            Bound::Place(place) => Some(place.clone()),
            _ => None,
        })
    }

    pub fn peek_audio(&self, fill: &Fill<V::Audio>) -> Option<V::Audio> {
        self.peek(fill, |value| match value {
            Bound::Audio(audio) => Some(audio.clone()),
            _ => None,
        })
    }

    pub fn place(&mut self, fill: &Fill<V::Place>) -> Result<V::Place, ScriptFault> {
        self.fill(fill, VariableType::Place, |value| match value {
            Bound::Place(place) => Some(place.clone()),
            _ => None,
        })
    }

    pub fn row(&mut self, fill: &Fill<V::Row>) -> Result<V::Row, ScriptFault> {
        self.fill(fill, VariableType::Row, |value| match value {
            Bound::Row(row) => Some(row.clone()),
            _ => None,
        })
    }

    pub fn audio(&mut self, fill: &Fill<V::Audio>) -> Result<V::Audio, ScriptFault> {
        self.fill(fill, VariableType::Audio, |value| match value {
            Bound::Audio(audio) => Some(audio.clone()),
            _ => None,
        })
    }

    pub fn var(&mut self, name: &str) -> Result<&Bound<'c, V>, ScriptFault> {
        let Some((name, _)) = self.variables.get_key_value(name) else {
            return Err(ScriptFault::new(
                "unknown variable",
                format!("{name}{}", nearest(name, self.variables.keys().copied())),
            ));
        };
        let name: &'c str = name;
        self.used_vars.insert(name);
        self.variables[name].1.as_ref().ok_or(ScriptFault::new(
            "missing value",
            format!("{name} has no value"),
        ))
    }

    fn expr_tracking(
        &mut self,
        expr: &'c Expr<V>,
        want: ValueType,
    ) -> Result<(Eval, bool), ScriptFault> {
        self.subject_read = false;
        let eval = self.expr(expr, want)?;
        Ok((eval, self.subject_read))
    }

    fn typed_tracking(
        &mut self,
        expr: &'c Expr<V>,
    ) -> Result<(Eval, ValueType, bool), ScriptFault> {
        self.subject_read = false;
        let (eval, ty) = self.typed(expr)?;
        Ok((eval, ty, self.subject_read))
    }

    fn event_field(&self, name: &str) -> Option<(usize, ValueType)> {
        self.event_fields
            .iter()
            .position(|(known, _)| *known == name)
            .map(|index| (index, self.event_fields[index].1))
    }

    fn local(&self, name: &str) -> Option<(usize, ValueType)> {
        self.locals
            .iter()
            .position(|(known, _)| *known == name)
            .map(|index| (index, self.locals[index].1))
    }

    fn state(&self, name: &str) -> Option<(usize, ValueType)> {
        self.state
            .iter()
            .position(|(known, _)| *known == name)
            .map(|index| (index, self.state[index].1))
    }

    pub fn typed(&mut self, expr: &'c Expr<V>) -> Result<(Eval, ValueType), ScriptFault> {
        if expr.depth() > expr::MAX_DEPTH {
            return Err(ScriptFault::new(
                "out of range",
                format!("expressions nest at most {} deep", expr::MAX_DEPTH),
            ));
        }
        if expr.size() > expr::MAX_NODES {
            return Err(ScriptFault::new(
                "out of range",
                format!("expressions hold at most {} nodes", expr::MAX_NODES),
            ));
        }
        self.typed_inner(expr)
    }

    fn typed_inner(&mut self, expr: &'c Expr<V>) -> Result<(Eval, ValueType), ScriptFault> {
        Ok(match expr {
            Expr::Bool(value) => (constant(Value::Bool(*value)), ValueType::Boolean),
            Expr::Int(value) => (constant(Value::Int(*value)), ValueType::Integer),
            Expr::Num(value) => (constant(Value::Num(*value)), ValueType::Number),
            Expr::Text(value) => (constant(Value::Text(value.clone())), ValueType::Text),
            Expr::Var(name) => match *self.var(name)? {
                Bound::Integer(value) => (constant(Value::Int(value)), ValueType::Integer),
                Bound::Number(value) => (constant(Value::Num(value)), ValueType::Number),
                Bound::Boolean(value) => (constant(Value::Bool(value)), ValueType::Boolean),
                Bound::Text(value) => (constant(Value::Text(value.to_owned())), ValueType::Text),
                Bound::Condition(condition) => (
                    self.expr(condition, ValueType::Boolean)?,
                    ValueType::Boolean,
                ),
                Bound::Name(_) | Bound::Place(_) | Bound::Row(_) | Bound::Audio(_) => {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("{name} is not a value an expression can read"),
                    ));
                }
            },
            Expr::Local(name) => {
                let (index, ty) = self
                    .local(name)
                    .ok_or_else(|| ScriptFault::new("unknown variable", format!("local {name}")))?;
                self.read_slots.insert(Slot::Local(index));
                (Box::new(move |_, cx: &FlowCx| cx.locals[index].clone()), ty)
            }
            Expr::State(name) => {
                let (index, ty) = self
                    .state(name)
                    .ok_or_else(|| ScriptFault::new("unknown variable", format!("state {name}")))?;
                self.read_slots.insert(Slot::State(index));
                (Box::new(move |_, cx: &FlowCx| cx.state[index].clone()), ty)
            }
            Expr::Event(name) => {
                let (index, ty) = self.event_field(name).ok_or_else(|| {
                    ScriptFault::new("unknown field", format!("the event has no field {name}"))
                })?;
                (Box::new(move |_, cx: &FlowCx| cx.event_field(index)), ty)
            }
            Expr::Query(query) => {
                let ty = query.value_type(self);
                if query.reads_subject() {
                    self.subject_read = true;
                }
                (query.compile(self)?, ty)
            }
            Expr::All(list) | Expr::Any(list) => {
                let all = matches!(expr, Expr::All(_));
                let parts = self.list(list, ValueType::Boolean)?;
                (
                    Box::new(move |world, cx| {
                        let mut result = all;
                        for part in &parts {
                            let held = part(world, cx).truthy();
                            if all && !held {
                                result = false;
                                break;
                            }
                            if !all && held {
                                result = true;
                                break;
                            }
                        }
                        Value::Bool(result)
                    }),
                    ValueType::Boolean,
                )
            }
            Expr::Not(of) => {
                let of = self.expr(of, ValueType::Boolean)?;
                (
                    Box::new(move |world, cx| Value::Bool(!of(world, cx).truthy())),
                    ValueType::Boolean,
                )
            }
            Expr::Equal(a, b) | Expr::NotEqual(a, b) => {
                let negate = matches!(expr, Expr::NotEqual(..));
                let (a, ta) = self.typed_inner(a)?;
                let (b, tb) = self.typed_inner(b)?;
                if !(ta.accepts(tb) || tb.accepts(ta)) {
                    return Err(ScriptFault::new(
                        "wrong type",
                        format!("comparing {} with {}", ta.name(), tb.name()),
                    ));
                }
                (
                    Box::new(move |world, cx| {
                        let a = a(world, cx);
                        let b = b(world, cx);
                        let equal = match (&a, &b) {
                            (Value::Int(a), Value::Int(b)) => a == b,
                            (Value::Text(a), Value::Text(b)) => a == b,
                            (Value::Bool(a), Value::Bool(b)) => a == b,
                            (Value::Actor(a), Value::Actor(b)) => a == b,
                            (Value::Point(a), Value::Point(b)) => a == b,
                            _ => a.as_num() == b.as_num(),
                        };
                        Value::Bool(equal != negate)
                    }),
                    ValueType::Boolean,
                )
            }
            Expr::Less(a, b) | Expr::AtMost(a, b) | Expr::More(a, b) | Expr::AtLeast(a, b) => {
                let (a, b, _) = self.numeric(a, b)?;
                let compare: fn(f64, f64) -> bool = match expr {
                    Expr::Less(..) => |a, b| a < b,
                    Expr::AtMost(..) => |a, b| a <= b,
                    Expr::More(..) => |a, b| a > b,
                    _ => |a, b| a >= b,
                };
                (
                    Box::new(move |world, cx| {
                        Value::Bool(compare(a(world, cx).as_num(), b(world, cx).as_num()))
                    }),
                    ValueType::Boolean,
                )
            }
            Expr::Add(a, b)
            | Expr::Subtract(a, b)
            | Expr::Multiply(a, b)
            | Expr::Min(a, b)
            | Expr::Max(a, b) => {
                let (a, b, integer) = self.numeric(a, b)?;
                let (on_ints, on_nums): Arithmetic = match expr {
                    Expr::Add(..) => (i64::saturating_add, |a, b| a + b),
                    Expr::Subtract(..) => (i64::saturating_sub, |a, b| a - b),
                    Expr::Multiply(..) => (i64::saturating_mul, |a, b| a * b),
                    Expr::Min(..) => (i64::min, f64::min),
                    _ => (i64::max, f64::max),
                };
                if integer {
                    (
                        Box::new(move |world, cx| {
                            Value::Int(on_ints(a(world, cx).as_int(), b(world, cx).as_int()))
                        }),
                        ValueType::Integer,
                    )
                } else {
                    (
                        Box::new(move |world, cx| {
                            Value::Num(on_nums(a(world, cx).as_num(), b(world, cx).as_num()))
                        }),
                        ValueType::Number,
                    )
                }
            }
            Expr::Divide(a, b) => {
                let (a, b, _) = self.numeric(a, b)?;
                (
                    Box::new(move |world, cx| {
                        let divisor = b(world, cx).as_num();
                        Value::Num(if divisor == 0.0 {
                            0.0
                        } else {
                            a(world, cx).as_num() / divisor
                        })
                    }),
                    ValueType::Number,
                )
            }
            Expr::Round(of) => {
                let of = self.expr(of, ValueType::Number)?;
                (
                    Box::new(move |world, cx| Value::Int(of(world, cx).as_num().round() as i64)),
                    ValueType::Integer,
                )
            }
            Expr::Join(list) => {
                if list.len() > expr::MAX_LIST {
                    return Err(ScriptFault::new(
                        "out of range",
                        format!("at most {} parts", expr::MAX_LIST),
                    ));
                }
                let mut parts = Vec::new();
                for part in list.iter() {
                    parts.push(self.typed_inner(part)?.0);
                }
                (
                    Box::new(move |world, cx| {
                        Value::Text(parts.iter().map(|part| part(world, cx).as_text()).collect())
                    }),
                    ValueType::Text,
                )
            }
        })
    }

    fn list(&mut self, list: &'c [Expr<V>], want: ValueType) -> Result<Vec<Eval>, ScriptFault> {
        if list.len() > expr::MAX_LIST {
            return Err(ScriptFault::new(
                "out of range",
                format!("at most {} parts", expr::MAX_LIST),
            ));
        }
        list.iter().map(|part| self.expr(part, want)).collect()
    }

    fn numeric(
        &mut self,
        a: &'c Expr<V>,
        b: &'c Expr<V>,
    ) -> Result<(Eval, Eval, bool), ScriptFault> {
        let (a, ta) = self.typed_inner(a)?;
        let (b, tb) = self.typed_inner(b)?;
        for ty in [ta, tb] {
            if !ValueType::Number.accepts(ty) {
                return Err(ScriptFault::new(
                    "wrong type",
                    format!("{} is not a number", ty.name()),
                ));
            }
        }
        Ok((a, b, ta == ValueType::Integer && tb == ValueType::Integer))
    }
}

fn constant(value: Value) -> Eval {
    Box::new(move |_, _| value.clone())
}
