use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::data::attention::Id as AttentionId;
use crate::systems::area::AreaTag;
use crate::systems::npc::Npc;
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
    pub on_plate: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: AttentionId,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NpcMarks {
    pub npc: Entity,
    pub marks: Vec<Mark>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[component(map_entities)]
pub struct Attention(pub Vec<NpcMarks>);

impl MapEntities for Attention {
    fn map_entities<E: EntityMapper>(&mut self, mapper: &mut E) {
        for marked in &mut self.0 {
            marked.npc = mapper.get_mapped(marked.npc);
        }
    }
}

impl Attention {
    pub fn of(&self, npc: Entity) -> &[Mark] {
        self.0
            .iter()
            .find(|marked| marked.npc == npc)
            .map_or(&[], |marked| marked.marks.as_slice())
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StatusBadges(pub Vec<AttentionId>);

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

pub fn update(world: &mut World, npcs: &mut QueryState<(Entity, &'static Npc, &'static AreaTag)>) {
    let marking = world.resource::<MarkSources>().0.clone();
    let badging = world.resource::<BadgeSources>().0.clone();
    let townsfolk: Vec<(Entity, crate::systems::area::Id)> = npcs
        .iter(world)
        .filter(|(_, npc, _)| npc.def.get().talk.is_some())
        .map(|(entity, _, tag)| (entity, tag.area))
        .collect();
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    for player in players {
        let area = world.get::<AreaTag>(player).map(|tag| tag.area);
        let attention = Attention(
            townsfolk
                .iter()
                .filter(|(npc, at)| {
                    Some(*at) == area
                        && !stat::is_dead(world, *npc)
                        && visibility::present(world, *npc, player)
                })
                .filter_map(|&(npc, _)| {
                    let mut marks: Vec<Mark> = marking
                        .iter()
                        .flat_map(|source| source(world, player, npc))
                        .collect();
                    marks.sort_by_key(|mark| mark.kind);
                    (!marks.is_empty()).then_some(NpcMarks { npc, marks })
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
