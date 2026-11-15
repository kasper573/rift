//! Everyone playing on a server, whichever of its worlds their character stands in.

use std::sync::Arc;

use arc_swap::ArcSwap;
use bevy_ecs::prelude::*;

use super::{Away, CharacterAccount, ClientId, Players};
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::area::AreaTag;
use crate::systems::movement;
use crate::systems::script::Place;

/// Everyone playing on the server, world by world, as they stood when its worlds last
/// conferred: one roster, which every world of the server holds.
#[derive(Resource, Clone, Default)]
pub struct ServerRoster(Arc<ArcSwap<Vec<Enrollment>>>);

/// The players whose characters stand in one world, in the order of their clients.
#[derive(Resource, Clone, Default)]
pub struct Enrollment(Arc<[Enrolled]>);

/// One player of the roster: who they are, where their character stands, and whether they are
/// at it.
#[derive(Clone, Debug, PartialEq)]
pub struct Enrolled {
    pub client: ClientId,
    /// The account the character is played for, when its client signed in as one.
    pub account: Option<AccountId>,
    pub name: String,
    pub place: Place,
    /// Whether the character stands in the world without its player.
    pub away: bool,
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

    /// The player whose character is played for `account`.
    pub fn account(&self, account: &AccountId) -> Option<Enrolled> {
        self.find(|enrolled| enrolled.account.as_ref() == Some(account))
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
                account: world
                    .get::<CharacterAccount>(character)
                    .map(|account| account.0.clone()),
                away: world.get::<Away>(character).is_some(),
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
