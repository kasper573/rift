//! Everyone playing on a server, whichever of its worlds their character stands in.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use arc_swap::ArcSwap;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::prelude::*;

use super::{Away, CharacterAccount, ClientId, PlayedBy};
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::account::AccountId;
use crate::systems::actor::Name;
use crate::systems::area::Place;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::area_instance::{AreaInstanceId, AreaInstanceWorld};
use crate::systems::movement::Position;

/// Everyone playing on the server, world by world, as they stood when its worlds last
/// conferred: one roster, which every world of the server holds.
#[derive(Resource, Clone, Default)]
pub struct ServerRoster(Arc<ArcSwap<Listing>>);

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

    /// Lists `worlds` as everyone who plays, one enrollment for each of the server's worlds in
    /// the order they confer, for every world that holds the roster.
    pub fn list(&self, worlds: Vec<Enrollment>) {
        let before = self.0.load().found.clone();
        let found = if before.finds(&worlds) {
            before
        } else {
            Arc::new(Found::of(&worlds))
        };
        self.0.store(Arc::new(Listing { worlds, found }));
    }

    /// Lets go of every world's enrollment, so each frees its own as it enrolls anew rather than
    /// the one that lists them freeing them all. Nobody is listed until the worlds are again.
    pub fn unlist(&self) {
        let found = self.0.load().found.clone();
        self.0.store(Arc::new(Listing {
            worlds: Vec::new(),
            found,
        }));
    }

    /// Shows `see` everyone who plays, world by world, with where they stand.
    pub fn each(&self, mut see: impl FnMut(&Enrolled, Pos<Meters>)) {
        for world in self.0.load().worlds.iter() {
            for (enrolled, at) in world.players.iter().zip(world.at.iter()) {
                see(enrolled, *at);
            }
        }
    }

    /// Whether who plays, as whom, in which area or at their character changed in any world
    /// since the worlds conferred before.
    pub fn moved(&self) -> bool {
        self.0.load().worlds.iter().any(|world| world.moved)
    }

    /// Where the character of `client` stood when the worlds last conferred.
    pub fn place_of(&self, client: ClientId) -> Option<Place> {
        let listing = self.0.load();
        let (world, index) = listing.find(client)?;
        let enrollment = &listing.worlds[world];
        Some(Place {
            area: enrollment.players[index].area,
            at: enrollment.at[index],
        })
    }

    /// Which of the server's worlds, in the order they confer, the character of `client` stands
    /// in.
    pub fn world_of(&self, client: ClientId) -> Option<usize> {
        self.0.load().find(client).map(|(world, _)| world)
    }

    /// The first player called `name`, whatever its case.
    pub fn named(&self, name: &str) -> Option<Enrolled> {
        self.0
            .load()
            .worlds
            .iter()
            .flat_map(|world| world.players.iter())
            .find(|enrolled| enrolled.name.eq_ignore_ascii_case(name))
            .cloned()
    }

    pub fn player(&self, client: ClientId) -> Option<Enrolled> {
        let listing = self.0.load();
        let (world, index) = listing.find(client)?;
        Some(listing.worlds[world].players[index].clone())
    }

    /// The player whose character is played for `account`.
    pub fn account(&self, account: &AccountId) -> Option<Enrolled> {
        let client = *self.0.load().found.accounts.get(account)?;
        self.player(client)
    }
}

/// What the roster lists: every world's enrollment, and where in them to find each player.
#[derive(Default)]
struct Listing {
    worlds: Vec<Enrollment>,
    found: Arc<Found>,
}

impl Listing {
    /// The world whose enrollment lists `client`, and their place in it.
    fn find(&self, client: ClientId) -> Option<(usize, usize)> {
        let &(world, index) = self.found.clients.get(&client)?;
        let listed = self.worlds.get(world)?.players.get(index)?;
        (listed.client == client).then_some((world, index))
    }
}

/// Where each player is listed: the world among the server's, and the place in its enrollment.
/// It holds as long as every world lists the players it did when they were found, which most
/// ticks they do, so it is found again only when someone came, went or changed.
#[derive(Default)]
struct Found {
    /// The players each world listed. Held weakly, so a list that is gone still can't be
    /// mistaken for one made in its place.
    of: Vec<Weak<[Enrolled]>>,
    clients: HashMap<ClientId, (usize, usize)>,
    accounts: HashMap<AccountId, ClientId>,
}

impl Found {
    fn of(worlds: &[Enrollment]) -> Found {
        let mut found = Found {
            of: worlds
                .iter()
                .map(|world| Arc::downgrade(&world.players))
                .collect(),
            ..Found::default()
        };
        for (slot, world) in worlds.iter().enumerate() {
            for (index, enrolled) in world.players.iter().enumerate() {
                found.clients.insert(enrolled.client, (slot, index));
                if let Some(account) = &enrolled.account {
                    found.accounts.insert(account.clone(), enrolled.client);
                }
            }
        }
        found
    }

    /// Whether `worlds` list exactly the players these were found among.
    fn finds(&self, worlds: &[Enrollment]) -> bool {
        self.of.len() == worlds.len()
            && self
                .of
                .iter()
                .zip(worlds)
                .all(|(of, world)| std::ptr::addr_eq(of.as_ptr(), Arc::as_ptr(&world.players)))
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
