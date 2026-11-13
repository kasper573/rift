//! What one world asks be done to a player whose character stands in another. A world can only
//! reach its own entities, so it leaves the errand to be carried when the server's worlds confer.

use bevy_ecs::prelude::*;

use super::ClientId;

/// Something to do to the character of `to`, in the world it stands in, and what the world that
/// asked does once it knows how that went.
pub struct Errand {
    pub to: ClientId,
    pub deed: ErrandDeed,
    pub then: ErrandAnswer,
}

/// What an errand does to its player's character. It runs in another world than the one that
/// made it, so it holds no entity of its own world, which would name some other entity there.
pub type ErrandDeed = Box<dyn FnOnce(&mut World, Entity) -> Result<(), String> + Send + Sync>;

/// What the world that asked for an errand does with how it went.
pub type ErrandAnswer = Box<dyn FnOnce(&mut World, ErrandOutcome) + Send + Sync>;

/// How an errand went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrandOutcome {
    Done,
    Refused(String),
    /// The player's character stood in no world, or had fallen.
    Unreached,
}

/// The errands a world has asked for since the server's worlds last conferred.
#[derive(Resource, Default)]
pub struct Errands(Vec<Errand>);

pub fn send(world: &mut World, errand: Errand) {
    world.resource_mut::<Errands>().0.push(errand);
}

/// The errands `world` asked for, in the order it asked, leaving it none.
pub fn take(world: &mut World) -> Vec<Errand> {
    world
        .get_resource_mut::<Errands>()
        .map(|mut errands| std::mem::take(&mut errands.0))
        .unwrap_or_default()
}
