use bevy_app::App;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::prelude::*;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::ImageRef;
use crate::core::chunk::{CHUNK_SIDE, ChunkIndex};
use crate::core::content::{ContentFile, ContentId, Fixture, Nearby};
use crate::core::par;
use crate::core::tick::{Cached, Planner};
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::AreaTag;
use crate::systems::interact::{self, Interactive};
use crate::systems::movement::Position;
use crate::systems::player::Players;
use crate::systems::{stat, visibility};

const REFRESH_ROUNDS: u32 = 3;
const MARKED_RANGE: Meters = Meters(visibility::VIEW_DISTANCE.0 + 4.0);

pub fn register(app: &mut App) {
    use bevy_replication::AppReplicationExt;
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

/// Marks what draws each player's attention and badges their status, a share of the players a
/// replication tick: what each is shown is worked out on every core from the world as the tick
/// found it, then set in the order the players were found.
#[derive(Default)]
pub struct MarkAttention {
    targets: Cached<(Entity, &'static AreaTag, &'static Position), With<Interactive>>,
    turn: u32,
    found: Vec<(Entity, Entity, AttentionId)>,
    changed: Vec<(Entity, Option<Attention>, Option<StatusBadges>)>,
}

impl Planner for MarkAttention {
    fn prepare(&mut self, world: &mut World) {
        self.targets.ready(world);
    }

    fn plan(&mut self, world: &World) {
        self.changed.clear();
        if !crate::systems::replication_tick(world) {
            return;
        }
        self.turn = (self.turn + 1) % REFRESH_ROUNDS;
        let turn = self.turn;
        let marking = &world.resource::<MarkSources>().0;
        let badging = &world.resource::<BadgeSources>().0;
        let mut players: Vec<Entity> = world
            .resource::<Players>()
            .0
            .values()
            .copied()
            .enumerate()
            .filter(|(index, _)| *index as u32 % REFRESH_ROUNDS == turn)
            .map(|(_, player)| player)
            .collect();
        players.sort_unstable();
        let targets = self.targets.query(world);
        let chunks = world.resource::<ChunkIndex>();
        let mut seen: Vec<(Entity, Entity)> = players
            .par_iter()
            .with_min_len(par::grain(players.len()))
            .flat_map_iter(|&player| {
                let standing = world
                    .get::<AreaTag>(player)
                    .zip(world.get::<Position>(player))
                    .map(|(tag, position)| (tag.area, position.pos));
                // The chunks are as of the last filing, a tick ago: a chunk of margin covers
                // whoever walked over a chunk's edge since.
                standing
                    .into_iter()
                    .flat_map(|(area, at)| {
                        chunks
                            .around(at, Meters(MARKED_RANGE.0 + CHUNK_SIDE.0))
                            .filter(move |&target| {
                                targets.get(target).is_ok_and(|(_, tag, standing)| {
                                    tag.area == area && standing.pos.distance(at) <= MARKED_RANGE
                                })
                            })
                    })
                    .filter(|&target| {
                        interact::interaction_of(world, target).is_some()
                            && !stat::is_dead(world, target)
                            && visibility::present(world, target, player)
                    })
                    .map(|target| (target, player))
                    .collect::<Vec<_>>()
                    .into_iter()
            })
            .collect();
        seen.sort_unstable();
        let found = &mut self.found;
        found.clear();
        found.par_extend(
            seen.par_chunk_by(|a, b| a.0 == b.0)
                .flat_map_iter(|viewed| {
                    let target = viewed[0].0;
                    let viewers: Vec<Entity> = viewed.iter().map(|&(_, player)| player).collect();
                    let mut marks = Vec::new();
                    for source in marking {
                        source(world, target, &viewers, &mut |player, mark| {
                            marks.push((player, target, mark));
                        });
                    }
                    marks.into_iter()
                }),
        );
        found.sort_unstable();
        found.dedup();
        let found = &self.found;
        self.changed = players
            .par_iter()
            .with_min_len(par::grain(players.len()))
            .map(|&player| {
                let mut attention = Attention::default();
                let from = found.partition_point(|&(marked, ..)| marked < player);
                for &(_, target, mark) in found[from..]
                    .iter()
                    .take_while(|&&(marked, ..)| marked == player)
                {
                    match attention.0.last_mut() {
                        Some(marked) if marked.target == target => marked.marks.push(mark),
                        _ => attention.0.push(MarkedTarget {
                            target,
                            marks: vec![mark],
                        }),
                    }
                }
                let badges = StatusBadges(
                    badging
                        .iter()
                        .filter_map(|source| source(world, player))
                        .collect(),
                );
                (
                    player,
                    (world.get::<Attention>(player) != Some(&attention)).then_some(attention),
                    (world.get::<StatusBadges>(player) != Some(&badges)).then_some(badges),
                )
            })
            .filter(|(_, attention, badges)| attention.is_some() || badges.is_some())
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        for (player, attention, badges) in self.changed.drain(..) {
            if let Some(attention) = attention {
                world.entity_mut(player).insert(attention);
            }
            if let Some(badges) = badges {
                world.entity_mut(player).insert(badges);
            }
        }
    }
}
