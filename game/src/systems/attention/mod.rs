pub mod render;

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

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Attention>()
        .replicate::<StatusBadges>()
        .init_resource::<MarkSources>()
        .init_resource::<BadgeSources>();
}

pub struct AttentionDef {
    pub icon: AssetRef,
    pub label: &'static str,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: AttentionId,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MarkedTarget {
    pub target: Entity,
    pub marks: Vec<Mark>,
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
    pub fn of(&self, target: Entity) -> &[Mark] {
        self.0
            .iter()
            .find(|marked| marked.target == target)
            .map_or(&[], |marked| marked.marks.as_slice())
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StatusBadges(pub Vec<AttentionId>);

pub fn plate_marks(world: &World, viewer: Option<Entity>, actor: Entity) -> Vec<Mark> {
    if let Some(badges) = world.get::<StatusBadges>(actor) {
        return badges
            .0
            .iter()
            .map(|&kind| Mark {
                kind,
                label: kind.get().label.to_owned(),
            })
            .collect();
    }
    viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .and_then(|attention| attention.of(actor).first().cloned())
        .into_iter()
        .collect()
}

pub type MarkSource = fn(&World, Entity, Entity) -> Vec<Mark>;

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
) {
    let marking = world.resource::<MarkSources>().0.clone();
    let badging = world.resource::<BadgeSources>().0.clone();
    let interactive: Vec<(Entity, crate::systems::area::Id)> = targets
        .iter(world)
        .filter(|(target, _)| interact::interaction_of(world, *target).is_some())
        .map(|(target, tag)| (target, tag.area))
        .collect();
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
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
                    let mut marks: Vec<Mark> = marking
                        .iter()
                        .flat_map(|source| source(world, player, target))
                        .collect();
                    marks.sort_by_key(|mark| mark.kind);
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
