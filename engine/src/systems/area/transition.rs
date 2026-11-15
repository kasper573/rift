use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use super::AreaId;
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::mount::{self, CrossingRide};
use crate::systems::npc::Npc;
use crate::systems::player::{CharacterState, Played, Players};
use crate::systems::script::{EventKind, RiftEvent, RiftInstances, area_owner};
use crate::systems::text::LineText;

#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: AreaId,
    pub dest: Pos<Meters>,
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
    pub state: CharacterState,
    /// The mount they rode out on, and where on it they sat.
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
    let mut travelers: Vec<Traveler> = crossings
        .into_iter()
        .zip(rides)
        .filter_map(|((entity, crossing), ride)| leave(world, entity, crossing, ride))
        .collect();
    // A driver arrives before whoever rode with them, who are seated beside them as they arrive.
    travelers.sort_by_key(|traveler| traveler.ride.map(|ride| ride.seat));
    travelers
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
    raise_area(world, EventKind::Entered, entity);
    if first_visit {
        raise_area(world, EventKind::Discovered, entity);
    }
    entity
}

pub fn raise_area(world: &mut World, kind: EventKind, subject: Entity) {
    let is_npc = world.get::<Npc>(subject).is_some();
    let owner = area_owner(world);
    world.resource_mut::<RiftInstances>().raise(
        owner,
        kind,
        Some(subject),
        Vec::new(),
        move |event| match event {
            RiftEvent::Entered { by } | RiftEvent::Left { by } => by.admits(is_npc),
            _ => true,
        },
    );
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
        state,
        ride,
    };
    raise_area(world, EventKind::Left, entity);
    world
        .resource_mut::<Players>()
        .0
        .remove(&traveler.played.client);
    world.despawn(entity);
    Some(traveler)
}
