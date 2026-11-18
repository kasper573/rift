use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::OnceLock;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::bind::Binder;
use super::event::Run;
use super::kinds::{Kinds, Slot};
use super::{
    BoxEnding, DialogueView, EffectDef, EventDef, FlowKey, GlobalPlayerId, PlaceDef, QueryDef,
    RunnerKind, RunnerSpec, WordsDef,
};

/// Everything the features of a game add to scripting: the effects, queries and events scripts are
/// written with, the kinds of thing that own scripts, and how scripts reach the players and the
/// dialogue window of the game. Each feature registers its own beside its mechanic, and the
/// engine installs the result once, as serde and schemars, which read and describe script files,
/// take no context to find it in.
pub struct Vocabulary {
    pub(super) effects: Kinds<dyn EffectDef>,
    pub(super) queries: Kinds<dyn QueryDef>,
    pub(super) events: Kinds<dyn EventDef>,
    pub(super) lines: Slot<dyn WordsDef>,
    pub(super) labels: Slot<dyn WordsDef>,
    pub(super) places: Slot<dyn PlaceDef>,
    runners: Vec<(RunnerKind, RunnerSpec)>,
    players: Option<PlayerHost>,
    dialogue: Option<DialogueHost>,
    binders: Vec<Binder>,
    extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

/// How scripts find the players of the game.
#[derive(Clone, Copy)]
pub struct PlayerHost {
    /// The player called the name whose character stands in the world.
    pub local: fn(&World, &str) -> Option<Entity>,
    /// The player called the name, in whichever of the server's worlds they play.
    pub global: fn(&World, &str) -> Option<GlobalPlayerId>,
    /// The player whose character the actor is.
    pub global_of: fn(&World, Entity) -> Option<GlobalPlayerId>,
}

/// How a script's dialogue is shown to its subject and closed again.
#[derive(Clone, Copy)]
pub struct DialogueHost {
    pub show: fn(&mut World, FlowKey, Entity, DialogueView),
    pub close: fn(&mut World, FlowKey, Entity, BoxEnding),
}

static INSTALLED: OnceLock<Vocabulary> = OnceLock::new();

impl Vocabulary {
    /// The vocabulary of the game, once `install` made it.
    pub fn get() -> &'static Vocabulary {
        INSTALLED
            .get()
            .expect("the script vocabulary is installed before a script is read or described")
    }

    /// Installs the vocabulary `register` makes, unless one is installed already.
    pub fn install(register: impl FnOnce(&mut Vocabulary)) -> &'static Vocabulary {
        INSTALLED.get_or_init(|| {
            let mut vocabulary = Vocabulary::core();
            register(&mut vocabulary);
            vocabulary
        })
    }

    fn core() -> Vocabulary {
        let mut vocabulary = Vocabulary {
            effects: Kinds::default(),
            queries: Kinds::default(),
            events: Kinds::default(),
            lines: Slot::default(),
            labels: Slot::default(),
            places: Slot::default(),
            runners: Vec::new(),
            players: None,
            dialogue: None,
            binders: Vec::new(),
            extensions: HashMap::new(),
        };
        vocabulary.event::<Run>("run");
        vocabulary.runner(
            RunnerKind::ANY,
            RunnerSpec::at(super::bind::any_site, |_, _| None),
        );
        vocabulary
    }

    pub fn effect<C>(&mut self, name: &'static str) -> &mut Vocabulary
    where
        C: EffectDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.effects.add::<C>(
            name,
            |effect| Box::new(effect),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    pub fn query<C>(&mut self, name: &'static str) -> &mut Vocabulary
    where
        C: QueryDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.queries.add::<C>(
            name,
            |query| Box::new(query),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    pub fn event<C>(&mut self, name: &'static str) -> &mut Vocabulary
    where
        C: EventDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.events.add::<C>(
            name,
            |event| Box::new(event),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    /// The type a dialogue line is written as.
    pub fn line<C>(&mut self) -> &mut Vocabulary
    where
        C: WordsDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.lines.fill::<C>(
            |line| Box::new(line),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    /// The type the label of a dialogue choice is written as.
    pub fn label<C>(&mut self) -> &mut Vocabulary
    where
        C: WordsDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.labels.fill::<C>(
            |label| Box::new(label),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    /// The type a place variable is given as.
    pub fn place<C>(&mut self) -> &mut Vocabulary
    where
        C: PlaceDef + Serialize + DeserializeOwned + JsonSchema,
    {
        self.places.fill::<C>(
            |place| Box::new(place),
            |held| {
                let held: &dyn Any = held;
                held.downcast_ref()
            },
        );
        self
    }

    pub fn runner(&mut self, kind: RunnerKind, spec: RunnerSpec) -> &mut Vocabulary {
        assert!(
            self.runner_named(kind.name()).is_none(),
            "two kinds of script owner are called {}",
            kind.name()
        );
        self.runners.push((kind, spec));
        self
    }

    pub fn players(&mut self, players: PlayerHost) -> &mut Vocabulary {
        self.players = Some(players);
        self
    }

    pub fn dialogue(&mut self, host: DialogueHost) -> &mut Vocabulary {
        self.dialogue = Some(host);
        self
    }

    /// Has `binder` compile the scripts of the rows it owns as the game loads.
    pub fn binder(&mut self, binder: Binder) -> &mut Vocabulary {
        self.binders.push(binder);
        self
    }

    /// What a feature keeps beside the vocabulary for the nodes it registers: its own registry of
    /// what it says about them.
    pub fn extension<E: Default + Any + Send + Sync>(&mut self) -> &mut E {
        self.extensions
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(E::default()))
            .downcast_mut()
            .expect("an extension is stored under its own type")
    }

    pub fn extension_of<E: Any>(&self) -> Option<&E> {
        self.extensions
            .get(&TypeId::of::<E>())
            .and_then(|held| held.downcast_ref())
    }

    pub fn runners(&self) -> impl Iterator<Item = RunnerKind> + '_ {
        self.runners.iter().map(|(kind, _)| *kind)
    }

    pub fn runner_named(&self, name: &str) -> Option<RunnerKind> {
        self.runners().find(|kind| kind.name() == name)
    }

    pub fn effect_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.effects.names()
    }

    pub fn query_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.queries.names()
    }

    pub fn event_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.events.names()
    }

    pub(super) fn spec(&self, kind: RunnerKind) -> &RunnerSpec {
        self.runners
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, spec)| spec)
            .unwrap_or_else(|| panic!("the script owner {} was never registered", kind.name()))
    }

    pub(super) fn players_host(&self) -> &PlayerHost {
        self.players
            .as_ref()
            .expect("a feature registers how scripts find players")
    }

    pub(super) fn dialogue_host(&self) -> &DialogueHost {
        self.dialogue
            .as_ref()
            .expect("a feature registers how dialogue is shown")
    }

    pub(super) fn binders(&self) -> &[Binder] {
        &self.binders
    }
}
