use std::any::{Any, TypeId};
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::kinds::registered_node;
use super::vocabulary::Vocabulary;
use super::{RunnerKind, ScriptFault, ValueType};

/// A cause that starts a flow: what happened, as the mechanic that raises it declares it. A
/// script answers it with an `on` node holding one, whose fields say which of its kind it
/// answers.
pub trait EventDef: dyn_clone::DynClone + Any + Send + Sync {
    /// The kinds of owner the event is raised for.
    fn owners(&self) -> &'static [RunnerKind];

    /// Whether whoever caused the event is the flow's subject.
    fn has_subject(&self) -> bool {
        true
    }

    /// The values the event hands the flow it starts, each by the name a script reads it by.
    fn fields(&self) -> &'static [(&'static str, ValueType)] {
        &[]
    }

    /// The rules the event's own values keep, beyond what the schema says.
    fn check(&self) -> Result<(), ScriptFault> {
        Ok(())
    }
}

dyn_clone::clone_trait_object!(EventDef);

registered_node! {
    /// An event any registered kind may be, as an `on` node holds it.
    ScriptEvent of EventDef, kinds events, tag ByEvent
}

impl ScriptEvent {
    pub fn kind(&self) -> EventKind {
        let held: &dyn Any = &*self.0;
        EventKind(held.type_id())
    }
}

/// Which sort of event an event is, whatever its fields say.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventKind(TypeId);

impl EventKind {
    pub fn of<E: EventDef>() -> EventKind {
        EventKind(TypeId::of::<E>())
    }

    /// The kind registered under `name`.
    pub fn named(name: &str) -> Option<EventKind> {
        Vocabulary::get().events.id_of(name).map(EventKind)
    }

    pub fn name(self) -> &'static str {
        Vocabulary::get()
            .events
            .name_of(self.0)
            .expect("an event kind is a registered one")
    }
}

impl fmt::Debug for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A script calling another starts the callee at its `run` event.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {}

impl EventDef for Run {
    fn owners(&self) -> &'static [RunnerKind] {
        &[RunnerKind::ANY]
    }
}
