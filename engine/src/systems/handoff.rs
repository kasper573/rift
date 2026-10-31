//! An actor that walks deep into the next cell changes hands: the world that owned it lets it go
//! and the cell's world takes it up under the same name, as it stands, within the tick, so the
//! actor keeps walking, fighting and running its scripts where it is rather than where it came
//! from. A ghost of it next door becomes it; its old world holds a ghost of it from the next
//! exchange on.

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_replicon::prelude::Replicated;

use crate::core::geometry::Radians;
use crate::core::math::Pos;
use crate::core::script::compile::ProcedureId;
use crate::core::script::value::Value;
use crate::core::units::Meters;
use crate::systems::WorldArea;
use crate::systems::actor::Actor;
use crate::systems::area::transition::Crossing;
use crate::systems::area::{AreaTag, Wild};
use crate::systems::combat::{Attackers, Attitude, Dead};
use crate::systems::dialogue::{self, ConversationSession};
use crate::systems::identity::{GlobalId, Named};
use crate::systems::item::{DropLife, DroppedItem, Reservation};
use crate::systems::mirror::Ghost;
use crate::systems::movement::{HeardMove, MoveTarget, Position};
use crate::systems::npc::summon::Summoned;
use crate::systems::npc::{self, Home, Npc, NpcId, Pack, Roams};
use crate::systems::player::{self, ArrivalHold, CharacterState, ClientId, PlayedBy, Players};
use crate::systems::reach::{ReachAct, ReachIntent, ReachTarget};
use crate::systems::shop::ShopVisit;
use crate::systems::stat::{self, Immortal, Stats};
use crate::systems::status::Statuses;
use crate::systems::visibility::Presence;
use crate::systems::worlds::{Wake, Worlds};

/// A player whose character changed hands this tick: its connection follows, from the world it
/// played in to the one its character now stands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moved {
    pub client: ClientId,
    pub from: usize,
    pub to: usize,
}

/// Asks the world to hand the entity over to world `0`, where someone wants to deal with it:
/// posted by a [`Pull`](crate::systems::mirror::Effect::Pull), taken up at the next handoff.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pulled(pub usize);

/// Hands every actor or drop of an awake world standing deep in another cell, or pulled by one, over to that
/// cell's world, and tells which players' connections must follow.
pub fn handoff(worlds: &Worlds, apps: &mut [Option<&mut World>], wake: &[Wake]) -> Vec<Moved> {
    if !worlds.has_borders() {
        return Vec::new();
    }
    let made: Vec<bool> = apps.iter().map(Option::is_some).collect();
    let mut moved = Vec::new();
    for from in (0..apps.len()).filter(|&from| wake[from].awake()) {
        let Some(world) = apps[from].as_deref_mut() else {
            continue;
        };
        for (to, carried) in departing(worlds, from, world, &made) {
            if let Carried::Player(player) = &carried {
                moved.push(Moved {
                    client: player.client,
                    from,
                    to,
                });
            }
            arrive(
                apps[to]
                    .as_deref_mut()
                    .expect("departures go to made worlds"),
                carried,
            );
        }
    }
    moved
}

/// An actor as it leaves one world for another: everything the next world needs to carry on
/// with it, with every entity it refers to by name.
enum Carried {
    Player(Box<PlayerCarry>),
    Npc(Box<NpcCarry>),
    Drop(DropCarry),
}

struct DropCarry {
    name: GlobalId,
    at: Pos<Meters>,
    dropped: DroppedItem,
    reservation: Option<Reservation>,
    presence: Option<Presence>,
    life: Option<DropLife>,
}

struct PlayerCarry {
    client: ClientId,
    name: GlobalId,
    state: CharacterState,
    heard: HeardMove,
    immortal: bool,
    stance: Stance,
}

struct NpcCarry {
    def: NpcId,
    name: GlobalId,
    pack: Pack,
    stats: Stats,
    attitude: Option<Attitude>,
    home: Option<Home>,
    roams: Option<Roams>,
    wild: bool,
    summoned: Option<Option<GlobalId>>,
    immortal: bool,
    scripts: HashMap<ProcedureId, Vec<Value>>,
    stance: Stance,
}

/// What any actor carries of the moment it changes hands in.
struct Stance {
    at: Pos<Meters>,
    facing: Radians,
    statuses: Option<Statuses>,
    attackers: Vec<GlobalId>,
    walk: Option<Pos<Meters>>,
    intent: Option<(CarriedTarget, ReachAct)>,
}

enum CarriedTarget {
    Actor(GlobalId),
    Point(Pos<Meters>),
}

/// What a world owns that may change hands: its actors and drops, where they stand and whether
/// they're pulled.
type Standing = QueryState<
    (Entity, &'static Position, Option<&'static Pulled>),
    (
        Or<(With<Actor>, With<DroppedItem>)>,
        With<GlobalId>,
        Without<Ghost>,
    ),
>;

/// The queries a departure scan runs, kept so a tick doesn't match archetypes afresh.
#[derive(Resource)]
struct Scans {
    standing: Standing,
    sessions: dialogue::Sessions,
    pulled: QueryState<Entity, With<Pulled>>,
}

impl FromWorld for Scans {
    fn from_world(world: &mut World) -> Scans {
        Scans {
            standing: world.query_filtered(),
            sessions: world.query(),
            pulled: world.query_filtered(),
        }
    }
}

/// Who leaves the world this tick, and for which: those standing deep in another cell or pulled
/// by one, but for a cell whose world isn't made yet, where they wait at the border.
fn departing(
    worlds: &Worlds,
    index: usize,
    world: &mut World,
    made: &[bool],
) -> Vec<(usize, Carried)> {
    world.get_resource_or_init::<Scans>();
    let (leaving, pulled) = world.resource_scope(|world, mut scans: Mut<Scans>| {
        let held = dialogue::counterparts(world, &mut scans.sessions);
        let pulled: Vec<Entity> = scans.pulled.iter(world).collect();
        let leaving: Vec<(Entity, usize)> = scans
            .standing
            .iter(world)
            .filter(|(entity, ..)| !held.contains(entity))
            .filter_map(|(entity, position, pulled)| {
                let to = worlds
                    .beyond(index, position.pos)
                    .or(pulled.map(|pulled| pulled.0))?;
                made[to].then_some((entity, to))
            })
            .collect();
        (leaving, pulled)
    });
    for entity in pulled {
        world.entity_mut(entity).remove::<Pulled>();
    }
    let leaving: Vec<(Entity, usize)> = leaving
        .into_iter()
        .filter(|&(entity, _)| !holds(world, entity))
        .collect();
    leaving
        .into_iter()
        .filter_map(|(entity, to)| Some((to, take(world, entity)?)))
        .collect()
}

/// Whether the actor is in the middle of something its world must see through: a session, a
/// crossing, an arrival, or its death.
fn holds(world: &World, entity: Entity) -> bool {
    world.get::<ConversationSession>(entity).is_some()
        || world.get::<ShopVisit>(entity).is_some()
        || world.get::<Crossing>(entity).is_some()
        || world.get::<ArrivalHold>(entity).is_some()
        || world.get::<Dead>(entity).is_some()
}

fn take(world: &mut World, entity: Entity) -> Option<Carried> {
    let name = *world.get::<GlobalId>(entity)?;
    if let Some(dropped) = world.get::<DroppedItem>(entity).cloned() {
        let carried = DropCarry {
            name,
            at: world.get::<Position>(entity)?.pos,
            dropped,
            reservation: world.get::<Reservation>(entity).cloned(),
            presence: world.get::<Presence>(entity).cloned(),
            life: world.get::<DropLife>(entity).copied(),
        };
        world.despawn(entity);
        return Some(Carried::Drop(carried));
    }
    let stance = stance_of(world, entity)?;
    let immortal = world.get::<Immortal>(entity).is_some();
    if let Some(npc) = world.get::<Npc>(entity) {
        let def = npc.def;
        let named = world.resource::<Named>();
        let carried = NpcCarry {
            def,
            name,
            pack: *world.get::<Pack>(entity)?,
            stats: stat::snapshot(world, entity),
            attitude: world.get::<Attitude>(entity).copied(),
            home: world.get::<Home>(entity).copied(),
            roams: world.get::<Roams>(entity).copied(),
            wild: world.get::<Wild>(entity).is_some(),
            summoned: world
                .get::<Summoned>(entity)
                .map(|summoned| summoned.with.and_then(|with| named.name(with))),
            immortal,
            scripts: npc::script_state(world, entity),
            stance,
        };
        npc::despawn(world, entity);
        return Some(Carried::Npc(Box::new(carried)));
    }
    let client = world.get::<PlayedBy>(entity)?.client;
    let carried = PlayerCarry {
        client,
        name,
        state: CharacterState::of(world, entity, world.get::<player::SpawnPoint>(entity)?.0)?,
        heard: world.get::<HeardMove>(entity).copied().unwrap_or_default(),
        immortal,
        stance,
    };
    world.resource_mut::<Players>().0.remove(&client);
    world.despawn(entity);
    Some(Carried::Player(Box::new(carried)))
}

fn stance_of(world: &World, entity: Entity) -> Option<Stance> {
    let named = world.resource::<Named>();
    Some(Stance {
        at: world.get::<Position>(entity)?.pos,
        facing: world.get::<Actor>(entity)?.facing,
        statuses: world.get::<Statuses>(entity).cloned(),
        attackers: world
            .get::<Attackers>(entity)
            .map(|attackers| {
                attackers
                    .ids
                    .iter()
                    .filter_map(|&id| named.name(id))
                    .collect()
            })
            .unwrap_or_default(),
        walk: world.get::<MoveTarget>(entity).map(|target| target.pos),
        intent: world.get::<ReachIntent>(entity).and_then(|intent| {
            let target = match intent.target {
                ReachTarget::Actor(actor) => CarriedTarget::Actor(named.name(actor)?),
                ReachTarget::Point(point) => CarriedTarget::Point(point),
            };
            Some((target, intent.act))
        }),
    })
}

fn arrive(world: &mut World, carried: Carried) {
    let area = world.resource::<WorldArea>().0;
    match carried {
        Carried::Npc(carry) => {
            let carry = *carry;
            let entity = under_name(world, carry.name);
            npc::furnish(
                world,
                entity,
                carry.def,
                carry.stance.at,
                area,
                carry.pack,
                carry.scripts,
            );
            world.entity_mut(entity).insert(carry.stats);
            if let Some(attitude) = carry.attitude {
                world.entity_mut(entity).insert(attitude);
            }
            if let Some(home) = carry.home {
                world.entity_mut(entity).insert(home);
            }
            if let Some(roams) = carry.roams {
                world.entity_mut(entity).insert(roams);
            }
            if carry.wild {
                world.entity_mut(entity).insert(Wild);
            }
            if let Some(with) = carry.summoned {
                let with = with.and_then(|with| world.resource::<Named>().get(with));
                world.entity_mut(entity).insert(Summoned { with });
            }
            if carry.immortal {
                world.entity_mut(entity).insert(Immortal);
            }
            stand(world, entity, carry.stance);
        }
        Carried::Drop(carry) => {
            let entity = under_name(world, carry.name);
            world.entity_mut(entity).insert((
                Replicated,
                Position::at(carry.at),
                AreaTag { area },
                carry.dropped,
            ));
            if let Some(reservation) = carry.reservation {
                world.entity_mut(entity).insert(reservation);
            }
            if let Some(presence) = carry.presence {
                world.entity_mut(entity).insert(presence);
            }
            if let Some(life) = carry.life {
                world.entity_mut(entity).insert(life);
            }
        }
        Carried::Player(carry) => {
            let carry = *carry;
            let entity = under_name(world, carry.name);
            player::furnish(
                world,
                entity,
                carry.client,
                area,
                carry.stance.at,
                carry.state,
            );
            world.entity_mut(entity).remove::<ArrivalHold>();
            world.entity_mut(entity).insert(carry.heard);
            if carry.immortal {
                world.entity_mut(entity).insert(Immortal);
            }
            stand(world, entity, carry.stance);
        }
    }
}

/// The entity to take the actor up under `name`: its ghost, no longer one, or a new one named so.
fn under_name(world: &mut World, name: GlobalId) -> Entity {
    match world.resource::<Named>().get(name) {
        Some(ghost) => {
            world.entity_mut(ghost).remove::<Ghost>();
            ghost
        }
        None => world.spawn(name).id(),
    }
}

fn stand(world: &mut World, entity: Entity, stance: Stance) {
    if let Some(mut actor) = world.get_mut::<Actor>(entity) {
        actor.facing = stance.facing;
    }
    if let Some(statuses) = stance.statuses {
        world.entity_mut(entity).insert(statuses);
    }
    let named = world.resource::<Named>();
    let attackers: Vec<Entity> = stance
        .attackers
        .iter()
        .filter_map(|&name| named.get(name))
        .collect();
    let intent = stance.intent.and_then(|(target, act)| {
        let target = match target {
            CarriedTarget::Actor(name) => ReachTarget::Actor(named.get(name)?),
            CarriedTarget::Point(point) => ReachTarget::Point(point),
        };
        Some(ReachIntent { target, act })
    });
    if !attackers.is_empty() {
        world
            .entity_mut(entity)
            .insert(Attackers { ids: attackers });
    }
    if let Some(intent) = intent {
        world.entity_mut(entity).insert(intent);
    }
    if let Some(pos) = stance.walk {
        world.entity_mut(entity).insert(MoveTarget { pos });
    }
}
