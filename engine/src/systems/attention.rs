use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::ImageRef;
use crate::core::content::{ContentFile, ContentId, Fixture, Nearby};
use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::interact::{self, Interactive};
use crate::systems::movement::Position;
use crate::systems::player::Players;
use crate::systems::{stat, visibility};

const REFRESH_ROUNDS: u32 = 3;
const MARKED_RANGE: Meters = Meters(visibility::VIEW_DISTANCE.0 + 4.0);

pub fn register(app: &mut App) {
    use crate::systems::mirror::AppMirrorExt;
    app.replicate_private::<Attention>()
        .replicate::<StatusBadges>();
}

pub type AttentionId = ContentId<AttentionDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct AttentionDef {
    pub icon: ImageRef,
}

impl ContentFile for AttentionDef {
    const TABLE: &'static str = "attention";

    fn fresh(near: &Nearby) -> Result<AttentionDef, String> {
        Ok(AttentionDef {
            icon: near.image()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum AttentionMark {
    QuestReady,
    QuestOffered,
    RepeatableReady,
    RepeatableOffered,
    QuestInProgress,
    QuestLocked,
    News,
    Talking,
}

impl Fixture for AttentionMark {
    type Row = AttentionDef;
    const FOLDER: Option<&'static str> = Some("attention");
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

pub type MarkSource = fn(&World, Entity, &[Entity], &mut dyn FnMut(Entity, AttentionId));

pub type BadgeSource = fn(&World, Entity) -> Option<AttentionId>;

#[derive(Resource, Default)]
pub struct MarkSources(Vec<MarkSource>);

#[derive(Resource, Default)]
pub struct BadgeSources(Vec<BadgeSource>);

pub fn mark_source(world: &mut World, source: MarkSource) {
    world.resource_mut::<MarkSources>().0.push(source);
}

pub fn badge_source(world: &mut World, source: BadgeSource) {
    world.resource_mut::<BadgeSources>().0.push(source);
}

pub fn update(
    world: &mut World,
    targets: &mut QueryState<(Entity, &'static AreaTag, &'static Position), With<Interactive>>,
    mut turn: Local<u32>,
    mut viewers: Local<Vec<Entity>>,
    mut found: Local<Vec<(Entity, Entity, AttentionId)>>,
) {
    *turn = (*turn + 1) % REFRESH_ROUNDS;
    let marking = world.resource::<MarkSources>().0.clone();
    let badging = world.resource::<BadgeSources>().0.clone();
    let players: Vec<(Entity, Option<AreaId>, Option<Pos<Meters>>)> = world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .enumerate()
        .filter(|(index, _)| *index as u32 % REFRESH_ROUNDS == *turn)
        .map(|(_, player)| {
            (
                player,
                world.get::<AreaTag>(player).map(|tag| tag.area),
                world.get::<Position>(player).map(|position| position.pos),
            )
        })
        .collect();
    found.clear();
    for (target, tag, position) in targets.iter(world) {
        if interact::interaction_of(world, target).is_none() || stat::is_dead(world, target) {
            continue;
        }
        let at = position.pos;
        viewers.clear();
        viewers.extend(
            players
                .iter()
                .filter(|(player, area, standing)| {
                    *area == Some(tag.area)
                        && standing.is_some_and(|standing| standing.distance(at) <= MARKED_RANGE)
                        && visibility::present(world, target, *player)
                })
                .map(|(player, ..)| *player),
        );
        if viewers.is_empty() {
            continue;
        }
        for source in &marking {
            source(world, target, &viewers, &mut |player, mark| {
                found.push((player, target, mark));
            });
        }
    }
    found.sort_unstable();
    found.dedup();
    let mut refreshed: Vec<(Entity, Attention)> = Vec::with_capacity(players.len());
    for &(player, target, mark) in found.iter() {
        let attention = match refreshed.last_mut() {
            Some((known, attention)) if *known == player => attention,
            _ => {
                refreshed.push((player, Attention::default()));
                &mut refreshed.last_mut().expect("just pushed").1
            }
        };
        match attention.0.last_mut() {
            Some(marked) if marked.target == target => marked.marks.push(mark),
            _ => attention.0.push(MarkedTarget {
                target,
                marks: vec![mark],
            }),
        }
    }
    for (player, ..) in players {
        let attention = refreshed
            .iter()
            .position(|(known, _)| *known == player)
            .map(|index| refreshed.swap_remove(index).1)
            .unwrap_or_default();
        if world.get::<Attention>(player) != Some(&attention) {
            world.entity_mut(player).insert(attention);
        }
        let badges = StatusBadges(
            badging
                .iter()
                .filter_map(|source| source(world, player))
                .collect(),
        );
        if world.get::<StatusBadges>(player) != Some(&badges) {
            world.entity_mut(player).insert(badges);
        }
    }
}
