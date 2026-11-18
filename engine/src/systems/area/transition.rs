use crate::systems::area::script::area_owner;
use std::collections::BTreeSet;

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AreaId;
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{EventDef, EventKind, Instances, RunnerKind, Vocabulary};
use crate::core::text::LineText;
use crate::core::units::Meters;
use crate::systems::area::AREA_RUNNER;
use crate::systems::area::object::{Entered, Left};
use crate::systems::area_instance::AreaInstanceWorld;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::mount::{self, CrossingRide};
use crate::systems::npc::Npc;
use crate::systems::player::{CharacterState, Played, Players};

#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: AreaId,
    pub dest: Pos<Meters>,
    /// The instance of the area they are bound for, or the world everyone there shares.
    pub instance: Option<AreaInstanceWorld>,
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Discovered(BTreeSet<AreaId>);

impl Discovered {
    pub fn starting_in(zone: AreaId) -> Discovered {
        Discovered(BTreeSet::from([zone]))
    }

    pub fn discover(&mut self, area: AreaId) -> bool {
        self.0.insert(area)
    }

    pub fn knows(&self, area: AreaId) -> bool {
        self.0.contains(&area)
    }

    pub fn areas(&self) -> impl Iterator<Item = AreaId> + '_ {
        self.0.iter().copied()
    }
}

pub struct Traveler {
    pub played: Played,
    pub dest_area: AreaId,
    pub dest: Pos<Meters>,
    /// The instance of the area they are bound for, or the world everyone there shares.
    pub instance: Option<AreaInstanceWorld>,
    pub state: CharacterState,
    /// The mount they rode out on.
    pub ride: Option<CrossingRide>,
}

pub fn departing(world: &mut World) -> Vec<Traveler> {
    let crossings: Vec<(Entity, Crossing)> = world
        .query::<(Entity, &Crossing)>()
        .iter(world)
        .map(|(entity, crossing)| (entity, *crossing))
        .collect();
    let rides: Vec<Option<CrossingRide>> = crossings
        .iter()
        .map(|(entity, _)| mount::crossing(world, *entity))
        .collect();
    crossings
        .into_iter()
        .zip(rides)
        .filter_map(|((entity, crossing), ride)| leave(world, entity, crossing, ride))
        .collect()
}

pub fn arrive(world: &mut World, mut traveler: Traveler) -> Entity {
    let content = world.resource::<Content>().clone();
    let first_visit = traveler.state.discovered.discover(traveler.dest_area);
    let entity = crate::systems::player::place(
        world,
        traveler.played,
        traveler.dest_area,
        traveler.dest,
        traveler.state,
    );
    if let Some(ride) = traveler.ride {
        mount::arrive(world, entity, ride);
    }
    let arrived = HistoryEntry::of(
        HistoryTopic::Notification,
        LineText::plain(&*traveler.dest_area.get(&content).name),
    )
    .mark(Some(HistoryMark::Arrived));
    history::record(world, entity, arrived);
    raise_area(world, EventKind::of::<Entered>(), entity);
    if first_visit {
        raise_area(world, EventKind::of::<AreaDiscovered>(), entity);
    }
    entity
}

pub fn raise_area(world: &mut World, kind: EventKind, subject: Entity) {
    let is_npc = world.get::<Npc>(subject).is_some();
    let owner = area_owner(world);
    world
        .resource_mut::<Instances>()
        .raise(owner, kind, Some(subject), Vec::new(), move |event| {
            event
                .downcast_ref::<Entered>()
                .map(|crossed| crossed.by)
                .or_else(|| event.downcast_ref::<Left>().map(|crossed| crossed.by))
                .is_none_or(|by| by.admits(is_npc))
        });
}

fn leave(
    world: &mut World,
    entity: Entity,
    crossing: Crossing,
    ride: Option<CrossingRide>,
) -> Option<Traveler> {
    let content = world.resource::<Content>().clone();
    let mut state = CharacterState::of(world, entity, crossing.dest)?;
    state.memory.leave_area(&content);
    let traveler = Traveler {
        played: Played::of(world, entity)?,
        dest_area: crossing.dest_area,
        dest: crossing.dest,
        instance: crossing.instance,
        state,
        ride,
    };
    raise_area(world, EventKind::of::<Left>(), entity);
    world
        .resource_mut::<Players>()
        .0
        .remove(&traveler.played.client);
    world.despawn(entity);
    Some(traveler)
}

/// A character comes to an area they have not been in before.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct AreaDiscovered {}

impl EventDef for AreaDiscovered {
    fn owners(&self) -> &'static [RunnerKind] {
        &[AREA_RUNNER]
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.event::<AreaDiscovered>("discovered");
}
