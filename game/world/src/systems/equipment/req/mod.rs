mod job;
mod level;
mod stat;

pub use job::JobRequirement;
pub use level::LevelRequirement;
pub use stat::StatRequirement;

use bevy_ecs::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

pub trait Requirement: Send + Sync {
    fn met(&self, world: &World, player: Entity) -> bool;
}

pub fn met(world: &World, player: Entity, requirements: &[Box<dyn Requirement>]) -> bool {
    requirements
        .iter()
        .all(|requirement| requirement.met(world, player))
}

pub fn parse<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Box<dyn Requirement>>, D::Error> {
    use serde::de::Error;
    serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?
        .into_iter()
        .map(|(name, args)| {
            let registered = lookup(&name)
                .ok_or_else(|| Error::custom(format!("unknown requirement '{name}'")))?;
            (registered.build)(args).map_err(Error::custom)
        })
        .collect()
}

struct Registered {
    name: &'static str,
    build: fn(serde_json::Value) -> Result<Box<dyn Requirement>, String>,
}

inventory::collect!(Registered);

fn build<R: Requirement + DeserializeOwned + 'static>(
    args: serde_json::Value,
) -> Result<Box<dyn Requirement>, String> {
    serde_json::from_value::<R>(args)
        .map(|requirement| Box::new(requirement) as Box<dyn Requirement>)
        .map_err(|error| error.to_string())
}

fn lookup(name: &str) -> Option<&'static Registered> {
    inventory::iter::<Registered>().find(|registered| registered.name == name)
}
