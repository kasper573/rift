mod aggressive;
mod defensive;
mod pacifist;
mod protective;

pub use aggressive::Aggressive;
pub use defensive::Defensive;
pub use pacifist::Pacifist;
pub use protective::Protective;

use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::math::{Direction, Pos, Rng};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::data;
use crate::systems::Character;
use crate::systems::actor::{self, Action, Actor, Hitbox, Rgba, set_action};
use crate::systems::area::{self, AreaTag};
use crate::systems::combat::{AttackTarget, Attackers};
use crate::systems::effect::{self, Effect, TimedEffects};
use crate::systems::item::Reservation;
use crate::systems::movement::{MoveTarget, Path, Position, forget, position};
use crate::systems::player::Players;
use crate::systems::stat::{self, Stat, StatKind, Stats};

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Npc>();
    effect::source(app, chase);
}

pub fn chase(world: &World, entity: Entity) -> Vec<Effect> {
    if world.get::<Npc>(entity).is_some() && world.get::<AttackTarget>(entity).is_some() {
        vec![Effect::Chasing]
    } else {
        Vec::new()
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Npc {
    pub def: data::npc::Id,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pack(pub u32);

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

pub struct NpcDef {
    pub display_name: &'static str,
    pub model: data::model::Id,
    pub tint: Rgba,
    pub ai: &'static dyn Ai,
    pub stats: &'static [Stat],
    pub aggro: Tiles,
    pub rewards: &'static [crate::systems::rewards::Reward],
}

pub fn spawn_all(world: &mut World) {
    let area_id = world.resource::<crate::systems::WorldArea>().0;
    let assets = world.resource::<AssetService>().clone();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for (group, spawn) in area_id.get().spawns.iter().enumerate() {
            for _ in 0..spawn.population {
                spawn_npc(
                    world,
                    &assets,
                    &mut rng,
                    area_id,
                    spawn.npc,
                    Pack(group as u32),
                );
            }
        }
    });
}

fn spawn_npc(
    world: &mut World,
    assets: &AssetService,
    rng: &mut Rng,
    area_id: area::Id,
    def: data::npc::Id,
    pack: Pack,
) {
    let area = assets.resolve(area_id.get().map, area::build_area);
    let at = random_walkable(rng, area).unwrap_or(area.spawn);
    spawn(world, def, at, area_id, pack);
}

pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Tiles>, area: area::Id) -> Entity {
    let assets = world.resource::<AssetService>().clone();
    let entity = world.spawn(character(&assets, def, at, area)).id();
    world.entity_mut(entity).insert(Stats(def.stats.to_vec()));
    entity
}

pub fn spawn(
    world: &mut World,
    def: data::npc::Id,
    at: Pos<Tiles>,
    area: area::Id,
    pack: Pack,
) -> Entity {
    let entity = spawn_actor(world, def.get(), at, area);
    world.entity_mut(entity).insert((Npc { def }, pack));
    entity
}

fn character(assets: &AssetService, def: &NpcDef, at: Pos<Tiles>, area: area::Id) -> Character {
    Character {
        replicated: Replicated,
        position: Position { pos: at },
        actor: Actor {
            color: def.tint,
            dir: Direction::S,
            action: Action::Idle,
            model: def.model,
            attack_rate: PlaybackRate(stat::value(def.stats, StatKind::AttackSpeed)),
        },
        hitbox: Hitbox {
            size: assets
                .resolve(*def.model.get(), actor::build_model)
                .hitbox(),
        },
        area: AreaTag { area },
    }
}

pub trait Ai: Send + Sync {
    fn wanders(&self, rng: &mut Rng) -> bool;
    fn target(&self, hunt: &Hunt) -> Option<Entity>;
}

pub struct Hunt<'a> {
    pub world: &'a World,
    pub players: &'a [Entity],
    pub enemies_by_pack: &'a HashMap<Pack, Vec<Entity>>,
    pub id: Entity,
    pub pack: Pack,
    pub at: Pos<Tiles>,
    pub area: area::Id,
    pub aggro: Tiles,
}

impl Hunt<'_> {
    pub fn nearest(
        &self,
        candidates: &[Entity],
        accept: impl Fn(Entity) -> bool,
    ) -> Option<Entity> {
        let mut best: Option<(Entity, Tiles)> = None;
        for &candidate in candidates {
            if stat::is_dead(self.world, candidate)
                || self.world.get::<AreaTag>(candidate).map(|t| t.area) != Some(self.area)
                || !accept(candidate)
            {
                continue;
            }
            if let Some(at) = position(self.world, candidate) {
                let distance = self.at.distance(at);
                if distance <= self.aggro && best.is_none_or(|(_, best)| distance < best) {
                    best = Some((candidate, distance));
                }
            }
        }
        best.map(|(entity, _)| entity)
    }
}

type NpcIds = QueryState<Entity, With<Npc>>;
type PackEnemies = QueryState<(&'static Pack, &'static Attackers)>;

pub fn run_ai(world: &mut World, npcs: &mut NpcIds, enemies: &mut PackEnemies) {
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    let assets = world.resource::<AssetService>().clone();
    let enemies_by_pack = enemies_by_pack(world, enemies);
    let ids: Vec<Entity> = npcs.iter(world).collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for id in ids {
            if stat::is_dead(world, id) {
                forget(world, id);
                continue;
            }
            let (Some(npc), Some(&pack)) = (world.get::<Npc>(id).copied(), world.get::<Pack>(id))
            else {
                continue;
            };
            let Some(at) = position(world, id) else {
                continue;
            };
            let def = npc.def.get();
            let Some(area) = world.get::<AreaTag>(id).map(|tag| tag.area) else {
                continue;
            };

            if let Some(target) = world.get::<AttackTarget>(id).map(|t| t.target) {
                if in_aggro(world, target, at, area, def.aggro) {
                    continue;
                }
                forget(world, id);
            }
            let target = {
                let hunt = Hunt {
                    world,
                    players: &players,
                    enemies_by_pack: &enemies_by_pack,
                    id,
                    pack,
                    at,
                    area,
                    aggro: def.aggro,
                };
                def.ai.target(&hunt)
            };
            if let Some(target) = target {
                world.entity_mut(id).insert(AttackTarget { target });
                continue;
            }
            idle_wander(world, &assets, &mut rng, id, def, area);
        }
    });
}

fn idle_wander(
    world: &mut World,
    assets: &AssetService,
    rng: &mut Rng,
    id: Entity,
    def: &NpcDef,
    area: area::Id,
) {
    if world.get::<MoveTarget>(id).is_some() || world.get::<Path>(id).is_some() {
        return;
    }
    if def.ai.wanders(rng)
        && let Some(at) = position(world, id)
        && let Some(node) =
            random_reachable(rng, assets.resolve(area.get().map, area::build_area), at)
    {
        world.entity_mut(id).insert(MoveTarget { pos: node });
    }
}

fn enemies_by_pack(world: &mut World, query: &mut PackEnemies) -> HashMap<Pack, Vec<Entity>> {
    let mut by_pack: HashMap<Pack, Vec<Entity>> = HashMap::new();
    for (&pack, attackers) in query.iter(world) {
        let list = by_pack.entry(pack).or_default();
        for attacker in &attackers.ids {
            if !list.contains(attacker) {
                list.push(*attacker);
            }
        }
    }
    by_pack
}

fn in_aggro(world: &World, target: Entity, at: Pos<Tiles>, area: area::Id, aggro: Tiles) -> bool {
    !stat::is_dead(world, target)
        && world.get::<AreaTag>(target).map(|t| t.area) == Some(area)
        && position(world, target).is_some_and(|p| at.distance(p) <= aggro)
}

pub fn run_respawn(world: &mut World, npcs: &mut NpcIds) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let ids: Vec<Entity> = npcs.iter(world).collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for id in ids {
            if !stat::is_dead(world, id) {
                world.entity_mut(id).remove::<DeadAt>();
                continue;
            }
            let since = match world.get::<DeadAt>(id) {
                Some(dead) => dead.at,
                None => {
                    world.entity_mut(id).insert(DeadAt { at: time });
                    time
                }
            };
            if time - since < NPC_RESPAWN_DELAY {
                continue;
            }
            let Some(region) = area::of(world, id) else {
                continue;
            };
            let at = random_walkable(&mut rng, region).unwrap_or(region.spawn);
            if let Some(mut position) = world.get_mut::<Position>(id) {
                position.pos = at;
            }
            if let Some(mut actor) = world.get_mut::<Actor>(id) {
                set_action(&mut actor, Action::Idle);
            }
            world
                .entity_mut(id)
                .remove::<DeadAt>()
                .remove::<Reservation>()
                .remove::<TimedEffects>();
            stat::refill(world, id);
            forget(world, id);
        }
    });
}

fn random_walkable(rng: &mut Rng, area: &area::Area) -> Option<Pos<Tiles>> {
    let nodes = &area.walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    Some(nodes[rng.rand_range(0..nodes.len() as u32) as usize])
}

fn random_reachable(rng: &mut Rng, area: &area::Area, from: Pos<Tiles>) -> Option<Pos<Tiles>> {
    let component_id = area.grid.component(from)?;
    let nodes = area
        .component_nodes
        .get(component_id as usize)
        .and_then(|n| if n.is_empty() { None } else { Some(n) })?;
    Some(nodes[rng.rand_range(0..nodes.len() as u32) as usize])
}
