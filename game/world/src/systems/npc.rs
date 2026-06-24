use std::collections::HashMap;
use std::sync::OnceLock;

use bevy_ecs::prelude::*;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use serde::Deserialize;

use crate::core::math::{Direction, Pos, Rng};
use crate::core::table;
use crate::core::table::{Content, Id};
use crate::core::tiling::{TilePos, Tiles, TilesPerSec};
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::systems::Character;
use crate::systems::actor::{Action, Actor, ActorModel, Hitbox, Name, Rgba, set_action};
use crate::systems::area::{self, AreaDef, AreaTag};
use crate::systems::combat::{AttackTarget, Attackers, Stats, Vitals, is_dead};
use crate::systems::movement::{MoveTarget, Path, Position, Speed, forget, position};
use crate::systems::player::Players;

const FILE: &str = "npc_table.json";
const SPAWN_FILE: &str = "spawn_table.json";

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);
const CHASE_SPEED_MULTIPLIER: f32 = 2.0;
const PACIFIST_WANDER_CHANCE: f32 = 0.4;
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Npc {
    pub def: Id<NpcDef>,
    /// Spawn-table row this NPC came from; protective NPCs defend others sharing their group.
    pub group: u32,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

#[derive(Resource)]
pub struct GameRng(pub Rng);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcDef {
    pub id: String,
    pub display_name: String,
    #[serde(deserialize_with = "Id::<ActorModel>::deserialize_named")]
    pub model: Id<ActorModel>,
    #[serde(deserialize_with = "crate::systems::actor::rgba_hex")]
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

impl Content for NpcDef {
    fn table() -> &'static [NpcDef] {
        defs()
    }
    fn id(&self) -> &str {
        &self.id
    }
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
    #[serde(deserialize_with = "Id::<NpcDef>::deserialize_named")]
    pub npc: Id<NpcDef>,
    #[serde(deserialize_with = "Id::<AreaDef>::deserialize_named")]
    pub area: Id<AreaDef>,
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

pub fn spawns() -> &'static [SpawnRow] {
    static SPAWNS: OnceLock<Vec<SpawnRow>> = OnceLock::new();
    SPAWNS.get_or_init(|| table::load(SPAWN_FILE))
}

pub fn spawn_all(world: &mut World) {
    let mut rng = Rng(RNG_SEED | 1);
    let area_id = world.resource::<crate::systems::WorldArea>().0;
    let area = &area::areas()[area_id.index()];
    for (group, row) in spawns().iter().enumerate() {
        if row.area != area_id {
            continue;
        }
        for _ in 0..row.population {
            spawn_npc(world, &mut rng, area, row.npc, group as u32);
        }
    }
    world.insert_resource(GameRng(rng));
}

fn spawn_npc(
    world: &mut World,
    rng: &mut Rng,
    area: &area::Area,
    def_index: Id<NpcDef>,
    group: u32,
) {
    let at = random_walkable(rng, area.id).unwrap_or(area.spawn);
    world.spawn((
        character(def_index.get(), at, area.id),
        Npc {
            def: def_index,
            group,
        },
    ));
}

/// The replicated [`Character`] bundle an [`NpcDef`] spawns as. Shared by the spawner and the
/// in-process benchmark so the two can't drift as `Character` or `NpcDef` gain fields.
pub fn character(def: &NpcDef, at: Pos<Tiles>, area: Id<AreaDef>) -> Character {
    Character {
        replicated: Replicated,
        position: Position { pos: at },
        name: Name {
            name: def.display_name.clone(),
        },
        actor: Actor {
            color: def.tint,
            dir: Direction::S,
            action: Action::Idle,
            model: def.model,
            attack_rate: def.attack_speed,
        },
        hitbox: Hitbox {
            size: def.model.get().hitbox(),
        },
        vitals: Vitals {
            health: def.health,
            max: def.health,
        },
        area: AreaTag { area },
        stats: Stats {
            damage: def.damage,
            attack_speed: def.attack_speed,
            attack_delay: def.attack_delay,
            range: def.range,
        },
        speed: Speed { value: def.speed },
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
        let def = npc.def.get();
        let area = world.get::<AreaTag>(id).map_or(Id::new(0), |tag| tag.area);

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

fn idle_wander(world: &mut World, rng: &mut Rng, id: Entity, def: &NpcDef, area: Id<AreaDef>) {
    if world.get::<MoveTarget>(id).is_some() || world.get::<Path>(id).is_some() {
        return;
    }
    let wander = match def.ai {
        Ai::Pacifist => rng.unit() < PACIFIST_WANDER_CHANCE,
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
    area: Id<AreaDef>,
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
    area: Id<AreaDef>,
    range: Tiles,
    accept: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    let mut best: Option<(Entity, Tiles)> = None;
    for &candidate in candidates {
        if is_dead(world, candidate)
            || world.get::<AreaTag>(candidate).map(|t| t.area) != Some(area)
            || !accept(candidate)
        {
            continue;
        }
        if let Some(p) = position(world, candidate) {
            let distance = at.distance(p);
            if distance <= range && best.is_none_or(|(_, b)| distance < b) {
                best = Some((candidate, distance));
            }
        }
    }
    best.map(|(entity, _)| entity)
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

fn in_aggro(
    world: &World,
    target: Entity,
    at: Pos<Tiles>,
    area: Id<AreaDef>,
    aggro: Tiles,
) -> bool {
    !is_dead(world, target)
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
        let area_id = world.get::<AreaTag>(id).map_or(Id::new(0), |tag| tag.area);
        let at = random_walkable(&mut rng, area_id)
            .unwrap_or_else(|| area::areas()[area_id.index()].spawn);
        if let Some(mut vitals) = world.get_mut::<Vitals>(id) {
            vitals.refill();
        }
        if let Some(mut position) = world.get_mut::<Position>(id) {
            position.pos = at;
        }
        if let Some(mut actor) = world.get_mut::<Actor>(id) {
            set_action(&mut actor, Action::Idle);
        }
        world.entity_mut(id).remove::<DeadAt>();
        forget(world, id);
        if let Some(speed) = world.get::<Npc>(id).map(|npc| npc.def.get().speed) {
            world.entity_mut(id).insert(Speed { value: speed });
        }
    }
    world.resource_mut::<GameRng>().0 = rng;
}

fn random_walkable(rng: &mut Rng, area_id: Id<AreaDef>) -> Option<Pos<Tiles>> {
    let nodes = &area::areas()[area_id.index()].walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    Some(nodes[(rng.roll() % nodes.len() as u64) as usize])
}
