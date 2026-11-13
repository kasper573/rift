//! Everyone playing on a server, whichever of its worlds their character stands in.

use std::sync::Arc;

use bevy_ecs::prelude::*;

use super::{ClientId, Players};
use crate::core::content::Content;
use crate::core::script::{ValueType, VariableType};
use crate::systems::actor::Name;
use crate::systems::area::AreaTag;
use crate::systems::movement;
use crate::systems::script::{Place, ScriptDef};

/// Everyone playing on the server, as they stood when its worlds last conferred. Only the worlds
/// of a game whose scripts take a global player hold one: any other game never asks who plays
/// beyond a world's own players.
#[derive(Resource, Clone, Default)]
pub struct ServerRoster(Arc<[Enrolled]>);

/// One player of the roster: who they are and where their character stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Enrolled {
    pub client: ClientId,
    pub name: String,
    pub place: Place,
}

impl ServerRoster {
    pub fn of(enrolled: Vec<Enrolled>) -> ServerRoster {
        ServerRoster(enrolled.into())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Enrolled> {
        self.0.iter()
    }

    /// The first player called `name`, whatever its case.
    pub fn named(&self, name: &str) -> Option<&Enrolled> {
        self.iter()
            .find(|enrolled| enrolled.name.eq_ignore_ascii_case(name))
    }

    pub fn player(&self, client: ClientId) -> Option<&Enrolled> {
        self.iter().find(|enrolled| enrolled.client == client)
    }
}

/// Whether any script of the game takes or keeps a global player, so its worlds must know who
/// plays in the others.
pub fn wanted(content: &Content) -> bool {
    content.table::<ScriptDef>().iter().any(|(_, script)| {
        script
            .variables
            .values()
            .any(|variable| variable.ty == VariableType::GlobalPlayer)
            || script
                .state
                .values()
                .chain(script.locals.values())
                .any(|ty| *ty == ValueType::GlobalPlayer)
    })
}

/// The players whose characters stand in `world`, in the order of their clients.
pub fn enrolled(world: &World) -> Vec<Enrolled> {
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
