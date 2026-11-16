//! Everyone playing on a server, whichever of its worlds their character stands in.

use std::sync::Arc;

use arc_swap::ArcSwap;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::prelude::*;

use super::{Away, CharacterAccount, ClientId, PlayedBy};
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::area_instance::{AreaInstanceId, AreaInstanceWorld};
use crate::systems::movement::Position;
use crate::systems::script::Place;

/// Everyone playing on the server, world by world, as they stood when its worlds last
/// conferred: one roster, which every world of the server holds.
#[derive(Resource, Clone, Default)]
pub struct ServerRoster(Arc<ArcSwap<Vec<Enrollment>>>);

/// The players whose characters stand in one world, in the order of their clients.
#[derive(Resource, Clone, Default)]
pub struct Enrollment {
    /// Who plays here. It is the list the enrollment before held for as long as nobody came,
    /// went or changed, so most ticks make none.
    players: Arc<[Enrolled]>,
    /// Where each of them stands, in their order.
    at: Arc<[Pos<Meters>]>,
    /// Whether who plays here differs from the world's enrollment before this one.
    moved: bool,
}

/// One player of the roster: who they are, the area their character stands in, and whether
/// they are at it.
#[derive(Clone, Debug, PartialEq)]
pub struct Enrolled {
    pub client: ClientId,
    /// The account the character is played for, when its client signed in as one.
    pub account: Option<AccountId>,
    pub name: String,
    pub area: AreaId,
    /// The instance their world is, unless it is one everyone in its area shares.
    pub instance: Option<AreaInstanceId>,
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

    /// Shows `see` everyone who plays, world by world.
    pub fn each(&self, mut see: impl FnMut(&Enrolled)) {
        for world in self.0.load().iter() {
            world.players.iter().for_each(&mut see);
        }
    }

    /// Whether who plays, as whom, in which area or at their character changed in any world
    /// since the worlds conferred before.
    pub fn moved(&self) -> bool {
        self.0.load().iter().any(|world| world.moved)
    }

    /// Where the character of `client` stood when the worlds last conferred.
    pub fn place_of(&self, client: ClientId) -> Option<Place> {
        self.0.load().iter().find_map(|world| {
            let listed = world
                .players
                .binary_search_by_key(&client, |enrolled| enrolled.client);
            let index = listed.ok()?;
            Some(Place {
                area: world.players[index].area,
                at: world.at[index],
            })
        })
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
            .flat_map(|world| world.players.iter())
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
    let instance = world.get_resource::<AreaInstanceWorld>().map(|run| run.id);
    let now = world.increment_change_tick();
    let enrollment = world.resource_scope(|world, mut query: Mut<EnrollmentQuery>| {
        let EnrollmentQuery {
            players,
            listed,
            since,
        } = &mut *query;
        let mut renamed = false;
        let mut standing: Vec<(Listed, Pos<Meters>)> = players
            .iter(world)
            .map(|(character, played, away, name, position)| {
                renamed |= since.is_none_or(|since| name.last_changed().is_newer_than(since, now));
                let listing = Listed {
                    client: played.client,
                    character,
                    away,
                    instance,
                };
                (listing, position.pos)
            })
            .collect();
        standing.sort_unstable_by_key(|(listing, _)| listing.client);
        let same = !renamed
            && standing.len() == listed.len()
            && standing
                .iter()
                .map(|(listing, _)| listing)
                .eq(listed.iter());
        *since = Some(now);
        if !same {
            *listed = standing.iter().map(|(listing, _)| *listing).collect();
        }
        Enrollment {
            players: if same {
                world.resource::<Enrollment>().players.clone()
            } else {
                listed
                    .iter()
                    .filter_map(|listing| listing.enrolled(world))
                    .collect()
            },
            at: standing.iter().map(|(_, at)| *at).collect(),
            moved: !same,
        }
    });
    *world.resource_mut::<Enrollment>() = enrollment.clone();
    enrollment
}

/// What finds a world's players and tells whether they are who they were, kept from one
/// enrollment to the next. Every world is enrolled on every tick, so nothing is read of a player
/// who plays on as they did but what stands side by side with everyone else's: whose they are,
/// that they are at their character, when their name last changed, and where they stand.
#[derive(Resource)]
pub struct EnrollmentQuery {
    players: QueryState<EnrolledPlayer>,
    /// Who was listed by the enrollment before, in its order.
    listed: Vec<Listed>,
    /// When the enrollment before was taken.
    since: Option<Tick>,
}

type EnrolledPlayer = (
    Entity,
    &'static PlayedBy,
    Has<Away>,
    Ref<'static, Name>,
    &'static Position,
);

/// What an enrollment tells one player by from the one before.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Listed {
    client: ClientId,
    character: Entity,
    away: bool,
    instance: Option<AreaInstanceId>,
}

impl Listed {
    /// The player as the roster lists them.
    fn enrolled(&self, world: &World) -> Option<Enrolled> {
        let character = world.get_entity(self.character).ok()?;
        Some(Enrolled {
            client: self.client,
            account: character
                .get::<CharacterAccount>()
                .map(|account| account.0.clone()),
            name: character.get::<Name>()?.name.clone(),
            area: character.get::<AreaTag>()?.area,
            instance: self.instance,
            away: self.away,
        })
    }
}

impl FromWorld for EnrollmentQuery {
    fn from_world(world: &mut World) -> EnrollmentQuery {
        EnrollmentQuery {
            players: world.query(),
            listed: Vec::new(),
            since: None,
        }
    }
}
