use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use super::AreaId;
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::history::{self, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::npc::Npc;
use crate::systems::player::{CharacterState, ClientId, PlayedBy, Players};
use crate::systems::script::{EventKind, RiftEvent, RiftInstances, area_owner};
use crate::systems::text::LineText;

#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: AreaId,
    pub dest: Pos<Meters>,
}

#[derive(Component, Clone, Debug, Default)]
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
}

pub struct Traveler {
    pub client: ClientId,
    pub dest_area: AreaId,
    pub dest: Pos<Meters>,
    pub state: CharacterState,
}

#[derive(Resource, Default)]
pub struct Departures(pub Vec<Traveler>);

pub fn departing(world: &mut World) -> Vec<Traveler> {
    let crossings: Vec<(Entity, Crossing)> = world
        .query::<(Entity, &Crossing)>()
        .iter(world)
        .map(|(entity, crossing)| (entity, *crossing))
        .collect();
    let mut leaving: Vec<Traveler> = crossings
        .into_iter()
        .filter_map(|(entity, crossing)| {
            leave(
                world,
                entity,
                crossing.dest_area,
                crossing.dest,
                crossing.dest,
            )
        })
        .collect();
    leaving.append(&mut world.resource_mut::<Departures>().0);
    leaving
}

pub fn leave(
    world: &mut World,
    entity: Entity,
    dest_area: AreaId,
    dest: Pos<Meters>,
    spawn: Pos<Meters>,
) -> Option<Traveler> {
    let content = world.resource::<Content>().clone();
    let mut state = CharacterState::of(world, entity, spawn)?;
    state.memory.leave_area(&content);
    let traveler = Traveler {
        client: world.get::<PlayedBy>(entity)?.client,
        dest_area,
        dest,
        state,
    };
    raise_area(world, EventKind::Left, entity);
    world.resource_mut::<Players>().0.remove(&traveler.client);
    world.despawn(entity);
    Some(traveler)
}

pub fn arrive(world: &mut World, mut traveler: Traveler) -> Entity {
    let content = world.resource::<Content>().clone();
    let first_visit = traveler.state.discovered.discover(traveler.dest_area);
    let entity = crate::systems::player::place(
        world,
        traveler.client,
        traveler.dest_area,
        traveler.dest,
        traveler.state,
    );
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
