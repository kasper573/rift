use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::data::attention::Id as AttentionId;
use crate::systems::area::AreaTag;
use crate::systems::interact::{self, Interactive};
use crate::systems::player::Players;
use crate::systems::stat;
use crate::systems::visibility;

const REFRESH_ROUNDS: u32 = 3;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Attention>()
        .replicate::<StatusBadges>()
        .init_resource::<MarkSources>()
        .init_resource::<BadgeSources>();
}

pub struct AttentionDef {
    pub icon: AssetRef,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MarkedTarget {
    pub target: Entity,
    pub marks: Vec<AttentionId>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[component(map_entities)]
pub struct Attention(pub Vec<MarkedTarget>);

impl MapEntities for Attention {
    fn map_entities<E: EntityMapper>(&mut self, mapper: &mut E) {
        for marked in &mut self.0 {
            marked.target = mapper.get_mapped(marked.target);
        }
    }
}

impl Attention {
    pub fn of(&self, target: Entity) -> &[AttentionId] {
        self.0
            .iter()
            .find(|marked| marked.target == target)
            .map_or(&[], |marked| marked.marks.as_slice())
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StatusBadges(pub Vec<AttentionId>);

pub fn plate_marks(world: &World, viewer: Option<Entity>, actor: Entity) -> Vec<AttentionId> {
    if let Some(badges) = world.get::<StatusBadges>(actor) {
        return badges.0.clone();
    }
    viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .and_then(|attention| attention.of(actor).first().cloned())
        .into_iter()
        .collect()
}

pub type MarkSource = fn(&World, Entity, Entity) -> Vec<AttentionId>;

pub type BadgeSource = fn(&World, Entity) -> Option<AttentionId>;

#[derive(Resource, Default)]
pub struct MarkSources(Vec<MarkSource>);

#[derive(Resource, Default)]
pub struct BadgeSources(Vec<BadgeSource>);

pub fn mark_source(app: &mut App, source: MarkSource) {
    app.world_mut().resource_mut::<MarkSources>().0.push(source);
}

pub fn badge_source(app: &mut App, source: BadgeSource) {
    app.world_mut()
        .resource_mut::<BadgeSources>()
        .0
        .push(source);
}

pub fn update(
    world: &mut World,
    targets: &mut QueryState<(Entity, &'static AreaTag), With<Interactive>>,
    mut turn: Local<u32>,
) {
    *turn = (*turn + 1) % REFRESH_ROUNDS;
    let marking = world.resource::<MarkSources>().0.clone();
    let badging = world.resource::<BadgeSources>().0.clone();
    let mut interactive: Vec<(Entity, crate::systems::area::Id)> = targets
        .iter(world)
        .filter(|(target, _)| interact::interaction_of(world, *target).is_some())
        .map(|(target, tag)| (target, tag.area))
        .collect();
    interactive.sort_by_key(|&(target, _)| target);
    let players: Vec<Entity> = world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .enumerate()
        .filter(|(index, _)| *index as u32 % REFRESH_ROUNDS == *turn)
        .map(|(_, player)| player)
        .collect();
    for player in players {
        let area = world.get::<AreaTag>(player).map(|tag| tag.area);
        let attention = Attention(
            interactive
                .iter()
                .filter(|(target, at)| {
                    Some(*at) == area
                        && !stat::is_dead(world, *target)
                        && visibility::present(world, *target, player)
                })
                .filter_map(|&(target, _)| {
                    let mut marks: Vec<AttentionId> = marking
                        .iter()
                        .flat_map(|source| source(world, player, target))
                        .collect();
                    marks.sort();
                    (!marks.is_empty()).then_some(MarkedTarget { target, marks })
                })
                .collect(),
        );
        let badges = StatusBadges(
            badging
                .iter()
                .filter_map(|source| source(world, player))
                .collect(),
        );
        if world.get::<Attention>(player) != Some(&attention) {
            world.entity_mut(player).insert(attention);
        }
        if world.get::<StatusBadges>(player) != Some(&badges) {
            world.entity_mut(player).insert(badges);
        }
    }
}
