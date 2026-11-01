use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::ImageRef;
use crate::core::chunk::{CHUNK_SIDE, ChunkIndex};
use crate::core::content::{ContentFile, ContentId, Fixture, Nearby};
use crate::core::math::Pos;
use crate::core::par;
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
    mut found: Local<Vec<(Entity, Entity, AttentionId)>>,
) {
    *turn = (*turn + 1) % REFRESH_ROUNDS;
    let marking = world.resource::<MarkSources>().0.clone();
    let badging = world.resource::<BadgeSources>().0.clone();
    let mut players: Vec<Entity> = world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .enumerate()
        .filter(|(index, _)| *index as u32 % REFRESH_ROUNDS == *turn)
        .map(|(_, player)| player)
        .collect();
    players.sort_unstable();
    let targets: Vec<(Entity, AreaId, Pos<Meters>)> = targets
        .iter(world)
        .map(|(target, tag, position)| (target, tag.area, position.pos))
        .collect();
    let shared: &World = world;
    let chunks = shared.resource::<ChunkIndex>();
    found.clear();
    found.par_extend(targets.par_iter().with_min_len(par::GRAIN).flat_map_iter(
        |&(target, area, at)| {
            let mut marks = Vec::new();
            if interact::interaction_of(shared, target).is_none() || stat::is_dead(shared, target) {
                return marks.into_iter();
            }
            // The chunks are as of the last filing, a tick ago: a chunk of margin covers
            // whoever walked over a chunk's edge since.
            let mut viewers: Vec<Entity> = chunks
                .around(at, Meters(MARKED_RANGE.0 + CHUNK_SIDE.0))
                .filter(|player| players.binary_search(player).is_ok())
                .filter(|&player| {
                    shared
                        .get::<AreaTag>(player)
                        .is_some_and(|tag| tag.area == area)
                        && shared
                            .get::<Position>(player)
                            .is_some_and(|standing| standing.pos.distance(at) <= MARKED_RANGE)
                        && visibility::present(shared, target, player)
                })
                .collect();
            viewers.sort_unstable();
            if viewers.is_empty() {
                return marks.into_iter();
            }
            for source in &marking {
                source(shared, target, &viewers, &mut |player, mark| {
                    marks.push((player, target, mark));
                });
            }
            marks.into_iter()
        },
    ));
    found.sort_unstable();
    found.dedup();
    let mut marked = found.iter().peekable();
    for player in players {
        let mut attention = Attention::default();
        while let Some(&(_, target, mark)) = marked.next_if(|(marked, ..)| *marked == player) {
            match attention.0.last_mut() {
                Some(marked) if marked.target == target => marked.marks.push(mark),
                _ => attention.0.push(MarkedTarget {
                    target,
                    marks: vec![mark],
                }),
            }
        }
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
