use std::sync::OnceLock;

use rift::{Builder, Ctx, Entity, Map, Wire, World};
use serde::{Deserialize, Deserializer};

use crate::core::actors::{self, ActorModelId};
use crate::core::area::{self, AreaId};
use crate::core::math::{
    Direction, Millis, PlaybackRate, Pos, Rng, Seconds, Tiles, TilesPerSec, next_rng, rng_unit,
};
use crate::core::protocol::{
    ACTION_IDLE, Actor, AreaTag, Name, Position, Rgba, Vitals, is_dead, position, set_action,
};
use crate::core::table;
use crate::features::combat::{AttackTarget, Attackers, Stats};
use crate::features::movement::{MoveTarget, Path, Speed, forget};
use crate::features::player::{Players, zone};

const FILE: &str = "npc_table.json";
const SPAWN_FILE: &str = "spawn_table.json";

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);
const CHASE_SPEED_MULTIPLIER: f32 = 2.0;
const PACIFIST_WANDER_CHANCE: f32 = 0.4;
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;

/// An npc definition's index in [`defs`]; content tables reference npcs by id.
#[derive(Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct NpcId(pub u16);

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Npc {
    pub def: NpcId,
    pub group: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

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
    pub model: ActorModelId,
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
    pub area: AreaId,
    pub population: u32,
}

pub fn spawner(b: &mut Builder) {
    b.start(spawn_all);
}
pub fn ai(b: &mut Builder) {
    b.system(run_ai);
}
pub fn respawn(b: &mut Builder) {
    b.system(run_respawn);
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

/// For benchmarks: pair with the feature set minus `npc::spawner`. Distributes `total` npcs
/// per area following the spawn table's population weights, so the ai mix matches the game.
pub fn spawn_npcs(world: &mut World, seed: u64, total: usize, area_ids: &[AreaId]) {
    if area_ids.is_empty() {
        return;
    }
    let mut rng = seed | 1;
    let base = area::defs().len() as u32;
    for (slot, &area_id) in area_ids.iter().enumerate() {
        let area = &area::areas()[area_id.0 as usize];
        let area_total = total / area_ids.len() + usize::from(slot < total % area_ids.len());
        let rows: Vec<(usize, &SpawnRow)> = spawns()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.area == AreaId(area_id.0 % base))
            .collect();
        let weight_total: f32 = rows.iter().map(|(_, row)| row.population as f32).sum();
        for &(group, row) in &rows {
            let count =
                ((row.population as f32 / weight_total) * area_total as f32).round() as usize;
            for _ in 0..count {
                spawn_npc(world, &mut rng, area, row.npc, group as u32);
            }
        }
    }
}

fn spawn_all(ctx: &mut Ctx) {
    // Each shard populates only the area it owns; a lone server (no cluster) uses the spawn zone.
    let zone = zone(ctx);
    let mut rng = RNG_SEED | 1;
    {
        let world = &mut ctx.server.world;
        let area = &area::areas()[zone.0 as usize];
        for (group, row) in spawns().iter().enumerate() {
            if row.area != zone {
                continue;
            }
            for _ in 0..row.population {
                spawn_npc(world, &mut rng, area, row.npc, group as u32);
            }
        }
    }
    ctx.res.insert(Rng(rng));
}

fn spawn_npc(world: &mut World, rng: &mut u64, area: &area::Area, def_index: NpcId, group: u32) {
    let def = def(def_index);
    let at = random_walkable(rng, area.id).unwrap_or(area.spawn);
    let entity = world.spawn();
    world.insert(entity, Position { pos: at });
    world.insert(
        entity,
        Name {
            name: def.display_name.clone(),
        },
    );
    world.insert(
        entity,
        Actor {
            color: def.tint,
            dir: Direction::S as u8,
            action: ACTION_IDLE,
            model: def.model,
            attack_rate: def.attack_speed,
        },
    );
    world.insert(entity, actors::model_hitbox(def.model));
    world.insert(
        entity,
        Vitals {
            health: def.health,
            max: def.health,
        },
    );
    world.insert(entity, AreaTag { area: area.id });
    world.insert(
        entity,
        Npc {
            def: def_index,
            group,
        },
    );
    world.insert(
        entity,
        Stats {
            damage: def.damage,
            attack_speed: def.attack_speed,
            attack_delay: def.attack_delay,
            range: def.range,
        },
    );
    world.insert(entity, Speed { value: def.speed });
}

fn run_ai(ctx: &mut Ctx) {
    let players: Vec<Entity> = ctx
        .res
        .get::<Players>()
        .map_or_else(Vec::new, |p| p.0.values().copied().collect());
    let mut rng = ctx.res.get::<Rng>().map_or(1, |r| r.0);
    {
        let world = &mut ctx.server.world;
        let by_group = enemies_by_group(world);
        for id in world.ids::<Npc>() {
            if is_dead(world, id) {
                forget(world, id);
                continue;
            }
            let Some(npc) = world.get::<Npc>(id) else {
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
                world.insert(id, Speed { value: def.speed });
            }
            if let Some(target) = find_aggro(world, &players, &by_group, id, &npc, def, at, area) {
                world.insert(id, AttackTarget { target });
                world.insert(
                    id,
                    Speed {
                        value: def.speed * CHASE_SPEED_MULTIPLIER,
                    },
                );
                continue;
            }
            idle_wander(world, &mut rng, id, def, area);
        }
    }
    ctx.res.insert(Rng(rng));
}

fn idle_wander(world: &mut World, rng: &mut u64, id: Entity, def: &NpcDef, area: AreaId) {
    if world.has::<MoveTarget>(id) || world.has::<Path>(id) {
        return;
    }
    let wander = match def.ai {
        Ai::Pacifist => rng_unit(rng) < PACIFIST_WANDER_CHANCE,
        _ => true,
    };
    if wander && let Some(node) = random_walkable(rng, area) {
        world.insert(id, MoveTarget { pos: node });
        world.insert(id, Speed { value: def.speed });
    }
}

#[allow(clippy::too_many_arguments)]
fn find_aggro(
    world: &World,
    players: &[Entity],
    by_group: &Map<u32, Vec<Entity>>,
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
fn enemies_by_group(world: &World) -> Map<u32, Vec<Entity>> {
    let mut by_group: Map<u32, Vec<Entity>> = Map::default();
    for (entity, attackers) in world.iter::<Attackers>() {
        if let Some(npc) = world.get::<Npc>(entity) {
            let list = by_group.entry(npc.group).or_default();
            for attacker in attackers.ids {
                if !list.contains(&attacker) {
                    list.push(attacker);
                }
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

fn run_respawn(ctx: &mut Ctx) {
    let time = Seconds(ctx.time);
    let mut rng = ctx.res.get::<Rng>().map_or(1, |r| r.0);
    {
        let world = &mut ctx.server.world;
        for id in world.ids::<Npc>() {
            if !is_dead(world, id) {
                world.remove::<DeadAt>(id);
                continue;
            }
            let since = match world.get::<DeadAt>(id) {
                Some(dead) => dead.at,
                None => {
                    world.insert(id, DeadAt { at: time });
                    time
                }
            };
            if time - since < NPC_RESPAWN_DELAY {
                continue;
            }
            let area = world.get::<AreaTag>(id).map_or(AreaId(0), |tag| tag.area);
            let at = random_walkable(&mut rng, area).unwrap_or_default();
            world.modify::<Vitals>(id, |v| v.health = v.max);
            world.modify::<Position>(id, |p| p.pos = at);
            set_action(world, id, ACTION_IDLE);
            world.remove::<DeadAt>(id);
            forget(world, id);
            if let Some(npc) = world.get::<Npc>(id) {
                world.insert(
                    id,
                    Speed {
                        value: def(npc.def).speed,
                    },
                );
            }
        }
    }
    ctx.res.insert(Rng(rng));
}

fn random_walkable(rng: &mut u64, area_id: AreaId) -> Option<Pos<Tiles>> {
    let nodes = &area::areas()[area_id.0 as usize].walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    let node = nodes[(next_rng(rng) % nodes.len() as u64) as usize];
    Some(node.map(|t| t + 0.5))
}
