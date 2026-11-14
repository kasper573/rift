//! Everyone playing on a server, whichever of its worlds their character stands in.

use std::sync::Arc;

use arc_swap::ArcSwap;
use bevy_ecs::prelude::*;

use super::{ClientId, Players};
use crate::core::content::Content;
use crate::core::script::{ValueType, VariableType};
use crate::systems::actor::Name;
use crate::systems::area::AreaTag;
use crate::systems::memory::MemoryDef;
use crate::systems::movement;
use crate::systems::script::{Place, ScriptDef};

/// Everyone playing on the server, world by world, as they stood when its worlds last
/// conferred: one roster, which every world of the server holds. Only the worlds of a game whose
/// scripts take a global player hold it: any other game never asks who plays beyond a world's
/// own players.
#[derive(Resource, Clone, Default)]
pub struct ServerRoster(Arc<ArcSwap<Vec<Enrollment>>>);

/// The players whose characters stand in one world, in the order of their clients.
#[derive(Resource, Clone, Default)]
pub struct Enrollment(Arc<[Enrolled]>);

/// One player of the roster: who they are and where their character stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Enrolled {
    pub client: ClientId,
    pub name: String,
    pub place: Place,
}

impl ServerRoster {
    /// Whether this is the roster `other` is, and not one of another server.
    pub fn is(&self, other: &ServerRoster) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Lists `worlds` as everyone who plays, for every world that holds the roster.
    pub fn list(&self, worlds: Vec<Enrollment>) {
        self.0.store(Arc::new(worlds));
    }

    pub fn all(&self) -> Vec<Enrolled> {
        self.0
            .load()
            .iter()
            .flat_map(|world| world.0.iter().cloned())
            .collect()
    }

    /// The first player called `name`, whatever its case.
    pub fn named(&self, name: &str) -> Option<Enrolled> {
        self.find(|enrolled| enrolled.name.eq_ignore_ascii_case(name))
    }

    pub fn player(&self, client: ClientId) -> Option<Enrolled> {
        self.find(|enrolled| enrolled.client == client)
    }

    fn find(&self, mut is: impl FnMut(&Enrolled) -> bool) -> Option<Enrolled> {
        self.0
            .load()
            .iter()
            .flat_map(|world| world.0.iter())
            .find(|enrolled| is(enrolled))
            .cloned()
    }
}

/// Whether the game's scripts take or keep a global player, or a memory holds one, so its worlds
/// must know who plays in the others.
pub fn wanted(content: &Content) -> bool {
    let scripted = content.table::<ScriptDef>().iter().any(|(_, script)| {
        script
            .variables
            .values()
            .any(|variable| variable.ty == VariableType::GlobalPlayer)
            || script
                .state
                .values()
                .chain(script.locals.values())
                .any(|ty| *ty == ValueType::GlobalPlayer)
    });
    let remembered = content
        .table::<MemoryDef>()
        .iter()
        .any(|(_, memory)| memory.ty == ValueType::GlobalPlayer);
    scripted || remembered
}

/// Enrolls the players of `world` as they stand now, in the `roster` the world holds from here
/// on. The world keeps its enrollment as well as handing it out, so the one before it ends here,
/// with the world, when no roster lists it.
pub fn enroll(world: &mut World, roster: &ServerRoster) -> Enrollment {
    if !world.resource::<ServerRoster>().is(roster) {
        world.insert_resource(roster.clone());
    }
    let enrollment = Enrollment(enrolled(world).into());
    *world.resource_mut::<Enrollment>() = enrollment.clone();
    enrollment
}

fn enrolled(world: &World) -> Vec<Enrolled> {
    let mut enrolled: Vec<Enrolled> = world
        .resource::<Players>()
        .0
        .iter()
        .filter_map(|(&client, &character)| {
            Some(Enrolled {
                client,
                name: world.get::<Name>(character)?.name.clone(),
                place: Place {
                    area: world.get::<AreaTag>(character)?.area,
                    at: movement::position(world, character)?,
                },
            })
        })
        .collect();
    enrolled.sort_by_key(|enrolled| enrolled.client);
    enrolled
}
