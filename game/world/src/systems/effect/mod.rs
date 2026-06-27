mod chasing;
mod stat_modifier;

pub use chasing::Chasing;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::time::Seconds;
use crate::systems::stat::StatSet;

inventory::collect!(&'static dyn Effect);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<TimedEffects>().init_resource::<Sources>();
    source(app, timed);
}

#[derive(Clone, Copy)]
pub struct EffectContext<'a> {
    pub world: &'a World,
    pub source: Entity,
    pub target: Entity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectId(&'static str);

impl Serialize for EffectId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EffectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        effect_id(&name).ok_or_else(|| serde::de::Error::custom(format!("unknown effect '{name}'")))
    }
}

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
    fn def(&self) -> &'static dyn Effect {
        lookup(self.effect.0).expect("a registered effect")
    }
}

pub fn command(effect: &impl Effect, args: &impl Serialize) -> EffectCommand {
    EffectCommand {
        effect: effect_id(effect.name()).expect("a registered effect"),
        args: postcard::to_allocvec(args).expect("args serialize"),
    }
}

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
            let effect =
                lookup(&name).ok_or_else(|| Error::custom(format!("unknown effect '{name}'")))?;
            let args = effect.encode(args).map_err(Error::custom)?;
            Ok(EffectCommand {
                effect: EffectId(effect.name()),
                args,
            })
        })
        .collect()
}

pub trait Effect: Send + Sync {
    fn name(&self) -> &str;
    fn icon(&self) -> Option<&str>;
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String>;
    fn compute(&self, ctx: &EffectContext, args: &[u8]) -> StatSet;
    fn describe(&self, ctx: &EffectContext, args: &[u8]) -> String {
        self.compute(ctx, args).describe()
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TimedEffects(pub Vec<TimedEffect>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TimedEffect {
    pub command: EffectCommand,
    pub until: Seconds,
}

pub type Source = fn(&World, Entity) -> Vec<EffectCommand>;

#[derive(Resource, Default)]
pub struct Sources(Vec<Source>);

pub fn source(app: &mut App, source: Source) {
    app.world_mut().resource_mut::<Sources>().0.push(source);
}

pub fn active_effects(world: &World, entity: Entity) -> Vec<EffectCommand> {
    world
        .resource::<Sources>()
        .0
        .iter()
        .flat_map(|source| source(world, entity))
        .collect()
}

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

fn encode_args<A: Serialize + DeserializeOwned>(
    args: serde_json::Value,
) -> Result<Vec<u8>, String> {
    let args: A = serde_json::from_value(args).map_err(|error| error.to_string())?;
    postcard::to_allocvec(&args).map_err(|error| error.to_string())
}

fn decode<A: DeserializeOwned>(bytes: &[u8]) -> A {
    postcard::from_bytes(bytes).expect("args were validated and encoded at load")
}

fn lookup(name: &str) -> Option<&'static dyn Effect> {
    inventory::iter::<&'static dyn Effect>()
        .copied()
        .find(|effect| effect.name() == name)
}

fn effect_id(name: &str) -> Option<EffectId> {
    lookup(name).map(|effect| EffectId(effect.name()))
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
