use std::collections::HashMap;
use std::sync::OnceLock;

use bevy_ecs::prelude::*;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use serde::{Deserialize, Deserializer};

use crate::core::actors::{self, ActorModelId};
use crate::core::area::{self, AreaId};
use crate::core::math::{
    Direction, Millis, PlaybackRate, Pos, Seconds, Tiles, TilesPerSec, next_rng, rng_unit,
};
use crate::core::protocol::{
    ACTION_IDLE, Actor, AreaTag, Name, Position, Rgba, Vitals, is_dead, position, set_action,
};
use crate::core::{protocol, table};
use crate::features::combat::{AttackTarget, Attackers, Stats};
use crate::features::movement::{MoveTarget, Path, Speed, forget};
use crate::features::player::Players;

const FILE: &str = "npc_table.json";
const SPAWN_FILE: &str = "spawn_table.json";

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);
const CHASE_SPEED_MULTIPLIER: f32 = 2.0;
const PACIFIST_WANDER_CHANCE: f32 = 0.4;
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;

/// An npc definition's index in [`defs`]; content tables reference npcs by id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct NpcId(pub u16);

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Npc {
    pub def: NpcId,
    pub group: u32,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

/// The world's wander/drop randomness, advanced by every roll.
#[derive(Resource)]
pub struct GameRng(pub u64);

impl<'de> Deserialize<'de> for NpcId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        defs()
            .iter()
            .position(|def| def.id == id)
            .map(|index| NpcId(index as u16))
            .ok_or_else(|| serde::de::Error::custom(format!("unknown npc '{id}'")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcDef {
    pub id: String,
    pub display_name: String,
    #[serde(deserialize_with = "actors::model_by_name")]
    pub model: ActorModelId,
    #[serde(deserialize_with = "protocol::rgba_hex")]
    pub tint: Rgba,
    pub ai: Ai,
    pub health: f32,
    pub damage: f32,
    pub attack_speed: PlaybackRate,
    pub attack_delay: Millis,
    pub range: Tiles,
    pub speed: TilesPerSec,
    pub aggro: Tiles,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Ai {
    Pacifist,
    Defensive,
    Aggressive,
    Protective,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnRow {
    pub npc: NpcId,
    #[serde(deserialize_with = "area::area_by_name")]
    pub area: AreaId,
    pub population: u32,
}

pub fn defs() -> &'static [NpcDef] {
    static DEFS: OnceLock<Vec<NpcDef>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let defs: Vec<NpcDef> = table::load(FILE);
        table::unique_ids(defs.iter().map(|def| def.id.as_str()), FILE);
        defs
    })
}

pub fn def(id: NpcId) -> &'static NpcDef {
    &defs()[id.0 as usize]
}

pub fn spawns() -> &'static [SpawnRow] {
    static SPAWNS: OnceLock<Vec<SpawnRow>> = OnceLock::new();
    SPAWNS.get_or_init(|| table::load(SPAWN_FILE))
}

// Each area seeds its own sequence, reproducing the layout from when every area was its own shard.
pub fn spawn_all(world: &mut World) {
    let mut rng = RNG_SEED | 1;
    for area in area::areas() {
        rng = RNG_SEED | 1;
        for (group, row) in spawns().iter().enumerate() {
            if row.area != area.id {
                continue;
            }
            for _ in 0..row.population {
                spawn_npc(world, &mut rng, area, row.npc, group as u32);
            }
        }
    }
    world.insert_resource(GameRng(rng));
}

fn spawn_npc(world: &mut World, rng: &mut u64, area: &area::Area, def_index: NpcId, group: u32) {
    let def = def(def_index);
    let at = random_walkable(rng, area.id).unwrap_or(area.spawn);
    world.spawn((
        Replicated,
        Position { pos: at },
        Name {
            name: def.display_name.clone(),
        },
        Actor {
            color: def.tint,
            dir: Direction::S as u8,
            action: ACTION_IDLE,
            model: def.model,
            attack_rate: def.attack_speed,
        },
        actors::model_hitbox(def.model),
        Vitals {
            health: def.health,
            max: def.health,
        },
        AreaTag { area: area.id },
        Npc {
            def: def_index,
            group,
        },
        Stats {
            damage: def.damage,
            attack_speed: def.attack_speed,
            attack_delay: def.attack_delay,
            range: def.range,
        },
        Speed { value: def.speed },
    ));
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
        if is_dead(world, id) {
            forget(world, id);
            continue;
        }
        let Some(npc) = world.get::<Npc>(id).cloned() else {
            continue;
        };
        let Some(at) = position(world, id) else {
            continue;
        };
        let def = def(npc.def);
        let area = world.get::<AreaTag>(id).map_or(AreaId(0), |tag| tag.area);

        if let Some(target) = world.get::<AttackTarget>(id).map(|t| t.target) {
            if in_aggro(world, target, at, area, def.aggro) {
                continue;
            }
            forget(world, id);
            world.entity_mut(id).insert(Speed { value: def.speed });
        }
        if let Some(target) = find_aggro(world, &players, &by_group, id, &npc, def, at, area) {
            world.entity_mut(id).insert((
                AttackTarget { target },
                Speed {
                    value: def.speed * CHASE_SPEED_MULTIPLIER,
                },
            ));
            continue;
        }
        idle_wander(world, &mut rng, id, def, area);
    }
    world.resource_mut::<GameRng>().0 = rng;
}

fn idle_wander(world: &mut World, rng: &mut u64, id: Entity, def: &NpcDef, area: AreaId) {
    if world.get::<MoveTarget>(id).is_some() || world.get::<Path>(id).is_some() {
        return;
    }
    let wander = match def.ai {
        Ai::Pacifist => rng_unit(rng) < PACIFIST_WANDER_CHANCE,
        _ => true,
    };
    if wander && let Some(node) = random_walkable(rng, area) {
        world
            .entity_mut(id)
            .insert((MoveTarget { pos: node }, Speed { value: def.speed }));
    }
}

#[allow(clippy::too_many_arguments)]
fn find_aggro(
    world: &World,
    players: &[Entity],
    by_group: &HashMap<u32, Vec<Entity>>,
    id: Entity,
    npc: &Npc,
    def: &NpcDef,
    at: Pos<Tiles>,
    area: AreaId,
) -> Option<Entity> {
    match def.ai {
        Ai::Pacifist => None,
        Ai::Aggressive => nearest(world, players, at, area, def.aggro, |_| true),
        Ai::Defensive => nearest(world, players, at, area, def.aggro, |player| {
            world
                .get::<Attackers>(id)
                .is_some_and(|a| a.ids.contains(&player))
        }),
        Ai::Protective => by_group
            .get(&npc.group)
            .and_then(|enemies| nearest(world, enemies, at, area, def.aggro, |_| true)),
    }
}

fn nearest(
    world: &World,
    candidates: &[Entity],
    at: Pos<Tiles>,
    area: AreaId,
    range: Tiles,
    accept: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    let mut best: Option<(Entity, f32)> = None;
    for &candidate in candidates {
        if is_dead(world, candidate)
            || world.get::<AreaTag>(candidate).map(|t| t.area) != Some(area)
            || !accept(candidate)
        {
            continue;
        }
        if let Some(p) = position(world, candidate) {
            let distance = at.distance(p);
            if distance <= range.0 && best.is_none_or(|(_, b)| distance < b) {
                best = Some((candidate, distance));
            }
        }
    }
    best.map(|(entity, _)| entity)
}

// Computed once per tick (O(N)) instead of rescanning all NPCs per protector.
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

fn in_aggro(world: &World, target: Entity, at: Pos<Tiles>, area: AreaId, aggro: Tiles) -> bool {
    !is_dead(world, target)
        && world.get::<AreaTag>(target).map(|t| t.area) == Some(area)
        && position(world, target).is_some_and(|p| at.distance(p) <= aggro.0)
}

pub fn run_respawn(world: &mut World) {
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let mut rng = world.resource::<GameRng>().0;
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Npc>>()
        .iter(world)
        .collect();
    for id in ids {
        if !is_dead(world, id) {
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
        let area = world.get::<AreaTag>(id).map_or(AreaId(0), |tag| tag.area);
        let at = random_walkable(&mut rng, area).unwrap_or_default();
        if let Some(mut vitals) = world.get_mut::<Vitals>(id) {
            vitals.health = vitals.max;
        }
        if let Some(mut position) = world.get_mut::<Position>(id) {
            position.pos = at;
        }
        if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_action(&mut actor, ACTION_IDLE);
        }
        world.entity_mut(id).remove::<DeadAt>();
        forget(world, id);
        if let Some(speed) = world.get::<Npc>(id).map(|npc| def(npc.def).speed) {
            world.entity_mut(id).insert(Speed { value: speed });
        }
    }
    world.resource_mut::<GameRng>().0 = rng;
}

fn random_walkable(rng: &mut u64, area_id: AreaId) -> Option<Pos<Tiles>> {
    let nodes = &area::areas()[area_id.0 as usize].walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    let node = nodes[(next_rng(rng) % nodes.len() as u64) as usize];
    Some(node.map(|t| t + 0.5))
}
