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
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;

use crate::core::math::{Direction, Pos, Rng};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::data;
use crate::systems::Character;
use crate::systems::actor::{Action, Actor, Hitbox, Name, Rgba, set_action};
use crate::systems::area::{self, AreaTag};
use crate::systems::combat::{AttackTarget, Attackers};
use crate::systems::effect::{self, Effect, TimedEffects};
use crate::systems::item::Reservation;
use crate::systems::movement::{MoveTarget, Path, Position, forget, position};
use crate::systems::player::Players;
use crate::systems::stat::{self, StatKind, Stats};

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;

pub fn register(app: &mut App) {
    effect::source(app, chase);
}

pub fn chase(world: &World, entity: Entity) -> Vec<Effect> {
    if world.get::<Npc>(entity).is_some() && world.get::<AttackTarget>(entity).is_some() {
        vec![Effect::Chasing]
    } else {
        Vec::new()
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Npc {
    pub def: data::npc::Id,
    pub group: u32,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

#[derive(Resource)]
pub struct GameRng(pub Rng);

pub struct NpcDef {
    pub display_name: &'static str,
    pub model: data::model::Id,
    pub tint: Rgba,
    pub ai: &'static dyn Ai,
    pub stats: Stats,
    pub aggro: Tiles,
    pub rewards: &'static [crate::systems::rewards::Reward],
}

pub fn spawn_all(world: &mut World) {
    let mut rng = Rng::new(RNG_SEED);
    let area_id = world.resource::<crate::systems::WorldArea>().0;
    for (group, spawn) in area_id.get().spawns.iter().enumerate() {
        for _ in 0..spawn.population {
            spawn_npc(world, &mut rng, area_id, spawn.npc, group as u32);
        }
    }
    world.insert_resource(GameRng(rng));
}

fn spawn_npc(world: &mut World, rng: &mut Rng, area_id: area::Id, def: data::npc::Id, group: u32) {
    let spawn_pos = area::area(area_id).spawn;
    let at = random_walkable(rng, area_id).unwrap_or(spawn_pos);
    spawn(world, def, at, area_id, group);
}

pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Tiles>, area: area::Id) -> Entity {
    let entity = world.spawn(character(def, at, area)).id();
    world.entity_mut(entity).insert(def.stats.clone());
    entity
}

pub fn spawn(
    world: &mut World,
    def: data::npc::Id,
    at: Pos<Tiles>,
    area: area::Id,
    group: u32,
) -> Entity {
    let entity = spawn_actor(world, def.get(), at, area);
    world.entity_mut(entity).insert(Npc { def, group });
    entity
}

fn character(def: &NpcDef, at: Pos<Tiles>, area: area::Id) -> Character {
    Character {
        replicated: Replicated,
        position: Position { pos: at },
        name: Name {
            name: def.display_name.to_owned(),
        },
        actor: Actor {
            color: def.tint,
            dir: Direction::S,
            action: Action::Idle,
            model: def.model,
            attack_rate: PlaybackRate(def.stats.get(StatKind::AttackSpeed)),
        },
        hitbox: Hitbox {
            size: def.model.get().hitbox(),
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
    pub by_group: &'a HashMap<u32, Vec<Entity>>,
    pub id: Entity,
    pub group: u32,
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

pub fn run_ai(world: &mut World) {
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    let mut rng = world.resource::<GameRng>().0;
    let by_group = enemies_by_group(world);
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Npc>>()
        .iter(world)
        .collect();
    for id in ids {
        if stat::is_dead(world, id) {
            forget(world, id);
            continue;
        }
        let Some(npc) = world.get::<Npc>(id).copied() else {
            continue;
        };
        let Some(at) = position(world, id) else {
            continue;
        };
        let def = npc.def.get();
        let Some(area) = area::of(world, id).map(|area| area.id) else {
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
                by_group: &by_group,
                id,
                group: npc.group,
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
        idle_wander(world, &mut rng, id, def, area);
    }
    world.resource_mut::<GameRng>().0 = rng;
}

fn idle_wander(world: &mut World, rng: &mut Rng, id: Entity, def: &NpcDef, area: area::Id) {
    if world.get::<MoveTarget>(id).is_some() || world.get::<Path>(id).is_some() {
        return;
    }
    if def.ai.wanders(rng)
        && let Some(node) = random_walkable(rng, area)
    {
        world.entity_mut(id).insert(MoveTarget { pos: node });
    }
}

fn enemies_by_group(world: &mut World) -> HashMap<u32, Vec<Entity>> {
    let mut by_group: HashMap<u32, Vec<Entity>> = HashMap::new();
    let mut query = world.query::<(&Npc, &Attackers)>();
    for (npc, attackers) in query.iter(world) {
        let list = by_group.entry(npc.group).or_default();
        for attacker in &attackers.ids {
            if !list.contains(attacker) {
                list.push(*attacker);
            }
        }
    }
    by_group
}

fn in_aggro(world: &World, target: Entity, at: Pos<Tiles>, area: area::Id, aggro: Tiles) -> bool {
    !stat::is_dead(world, target)
        && world.get::<AreaTag>(target).map(|t| t.area) == Some(area)
        && position(world, target).is_some_and(|p| at.distance(p) <= aggro)
}

pub fn run_respawn(world: &mut World) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let mut rng = world.resource::<GameRng>().0;
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Npc>>()
        .iter(world)
        .collect();
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
        let region_id = region.id;
        let spawn_pos = region.spawn;
        let at = random_walkable(&mut rng, region_id).unwrap_or(spawn_pos);
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
    world.resource_mut::<GameRng>().0 = rng;
}

fn random_walkable(rng: &mut Rng, area_id: area::Id) -> Option<Pos<Tiles>> {
    let nodes = &area::area(area_id).walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    Some(nodes[rng.rand_range(0..nodes.len() as u32) as usize])
}
