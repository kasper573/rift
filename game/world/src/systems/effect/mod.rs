//! Effects: the source-code-driven effect "table". Each effect is one struct per file here
//! implementing the [`Effect`] trait, dispatched by `enum_dispatch` — there is no
//! `effect_table.json` because effects are code. Assets never embed an effect, only an
//! [`EffectCommand`] (an effect's struct-name id plus its parameters), validated against the chosen
//! effect when the table loads. An effect just `compute`s an immutable
//! [`StatSet`](crate::systems::stat::StatSet); the stats system sums it. Effects are condition-free;
//! each *condition* (equipped, carried, level, timed, an npc's chase) is a [`Source`] its own feature
//! registers, so the effect module names none of them.

mod chasing;
mod stat_modifier;

pub use chasing::Chasing;
pub use stat_modifier::StatModifier;

use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use enum_dispatch::enum_dispatch;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use strum::{EnumIter, IntoEnumIterator};

use crate::core::time::Seconds;
use crate::systems::stat::StatSet;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<TimedEffects>().init_resource::<Sources>();
    // Timed effects live here, so their source does too; every other source is registered by the
    // feature that owns its condition (gear, carrying, level, an npc's chase).
    source(app, timed);
}

/// The actors an effect is computed against — read access only, so it returns a predictable set of
/// stats and the stats system stays the one that applies them. For a self-buff `source == target`.
#[derive(Clone, Copy)]
pub struct EffectContext<'a> {
    pub world: &'a World,
    pub source: Entity,
    pub target: Entity,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectId(u32);

/// An effect chosen by id plus its parameters. Args are kept already-encoded, so a command is one
/// uniform, replicable value no matter which effect it names.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EffectCommand {
    effect: EffectId,
    args: Vec<u8>,
}

impl EffectCommand {
    pub fn icon(&self) -> Option<&'static str> {
        self.def().icon()
    }
    pub fn compute(&self, ctx: &EffectContext) -> StatSet {
        self.def().compute(ctx, &self.args)
    }
    pub fn describe(&self, ctx: &EffectContext) -> String {
        self.def().describe(ctx, &self.args)
    }
    fn def(&self) -> &'static EffectKind {
        &registry()[self.effect.0 as usize]
    }
}

/// Builds a command for a known effect, for systems that raise effects in code (an npc's chase)
/// rather than from a table.
pub fn command(effect: &impl Effect, args: &impl Serialize) -> EffectCommand {
    EffectCommand {
        effect: effect_id(effect.name()).expect("a registered effect"),
        args: postcard::to_allocvec(args).expect("args serialize"),
    }
}

/// Table-field reader: turns `[{ "StatModifier": { "stat": "Damage", "amount": 3 } }]` into validated
/// commands — the single key is the effect, the value its args — panicking through the loader if an
/// effect is unknown or its args don't fit it.
pub fn commands<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<EffectCommand>, D::Error> {
    use serde::de::Error;
    Vec::<serde_json::Map<String, serde_json::Value>>::deserialize(deserializer)?
        .into_iter()
        .map(|spec| {
            let mut entries = spec.into_iter();
            let (name, args) = entries
                .next()
                .ok_or_else(|| Error::custom("effect command must name one effect"))?;
            if entries.next().is_some() {
                return Err(Error::custom("effect command must name exactly one effect"));
            }
            let effect = effect_id(&name)
                .ok_or_else(|| Error::custom(format!("unknown effect '{name}'")))?;
            let args = registry()[effect.0 as usize]
                .encode(args)
                .map_err(Error::custom)?;
            Ok(EffectCommand { effect, args })
        })
        .collect()
}

/// The interface every effect file implements, dispatched by [`EffectKind`]. Args ride as json at
/// table load and postcard bytes at runtime; an effect validates+stores them with [`encode_args`] and
/// reads them back with [`decode`]. It only reads `ctx`; applying the stats is the stats system's job.
#[enum_dispatch]
pub trait Effect {
    fn name(&self) -> &str;
    fn icon(&self) -> Option<&str>;
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String>;
    fn compute(&self, ctx: &EffectContext, args: &[u8]) -> StatSet;
    /// Tooltip text; the default reads the computed stat delta, which suits any plain modifier. An
    /// effect with a richer story overrides it.
    fn describe(&self, ctx: &EffectContext, args: &[u8]) -> String {
        self.compute(ctx, args).describe()
    }
}

/// Timed effect instances on an actor, each lasting until its deadline. Any system can add to this;
/// [`expire`] prunes finished ones. Replicated so the client can show a player's active effects.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TimedEffects(pub Vec<TimedEffect>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TimedEffect {
    pub command: EffectCommand,
    pub until: Seconds,
}

/// A feature's contributor of an actor's active effect commands — equipped gear, a level, an npc's
/// chase, a timed buff. Each feature registers its own via [`source`]; the effect module knows none
/// of them specifically, so a new condition is a new source in its own feature, not a patch here.
pub type Source = fn(&World, Entity) -> Vec<EffectCommand>;

#[derive(Resource, Default)]
pub struct Sources(Vec<Source>);

/// Registers an effect source. Call from a feature's `register`, after [`register`] has created the
/// resource (it runs early in `protocol`).
pub fn source(app: &mut App, source: Source) {
    app.world_mut().resource_mut::<Sources>().0.push(source);
}

/// Every effect command currently active on `entity`, summed from the registered [`Source`]s. Pure
/// read, so the server (for combat) and the client (for the local player's UI) get identical results.
pub fn active_effects(world: &World, entity: Entity) -> Vec<EffectCommand> {
    world
        .resource::<Sources>()
        .0
        .iter()
        .flat_map(|source| source(world, entity))
        .collect()
}

/// Drops finished timed effects; effective stats are read fresh, so nothing else need happen.
pub fn expire(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<TimedEffects>>()
        .iter(world)
        .collect();
    for id in ids {
        if let Some(mut timed) = world.get_mut::<TimedEffects>(id)
            && timed.0.iter().any(|effect| effect.until <= now)
        {
            timed.0.retain(|effect| effect.until > now);
        }
    }
}

/// Every effect, one variant per file. `enum_dispatch` forwards [`Effect`] to the variant; `EnumIter`
/// lets [`registry`] build itself, so adding an effect is just a new file plus a variant here.
#[enum_dispatch(Effect)]
#[derive(EnumIter)]
enum EffectKind {
    StatModifier(StatModifier),
    Chasing(Chasing),
}

/// Validates `args` (json from a table) against `A` and re-encodes them as the postcard bytes an
/// [`EffectCommand`] stores. An effect's `encode` is a one-liner naming its args type.
fn encode_args<A: Serialize + DeserializeOwned>(
    args: serde_json::Value,
) -> Result<Vec<u8>, String> {
    let args: A = serde_json::from_value(args).map_err(|error| error.to_string())?;
    postcard::to_allocvec(&args).map_err(|error| error.to_string())
}

/// Reads back args an effect stored with [`encode_args`] or [`command`].
fn decode<A: DeserializeOwned>(bytes: &[u8]) -> A {
    postcard::from_bytes(bytes).expect("args were validated and encoded at load")
}

fn effect_id(name: &str) -> Option<EffectId> {
    registry()
        .iter()
        .position(|effect| effect.name() == name)
        .map(|index| EffectId(index as u32))
}

fn registry() -> &'static [EffectKind] {
    static REGISTRY: OnceLock<Vec<EffectKind>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let effects: Vec<EffectKind> = EffectKind::iter().collect();
        crate::core::table::unique_ids(effects.iter().map(|effect| effect.name()), "effects");
        effects
    })
}

fn timed(world: &World, entity: Entity) -> Vec<EffectCommand> {
    world
        .get::<TimedEffects>(entity)
        .map(|timed| {
            timed
                .0
                .iter()
                .map(|effect| effect.command.clone())
                .collect()
        })
        .unwrap_or_default()
}
