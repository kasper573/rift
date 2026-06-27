mod aggressive;
mod defensive;
mod pacifist;
mod protective;

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use serde::{Deserialize, Deserializer};

use crate::core::math::{Direction, Pos, Rng};
use crate::core::table;
use crate::core::table::{Content, Id};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::systems::Character;
use crate::systems::actor::{Action, Actor, ActorModel, Hitbox, Name, Rgba, set_action};
use crate::systems::area::{self, AreaDef, AreaTag};
use crate::systems::combat::{AttackTarget, Attackers};
use crate::systems::effect::Chasing;
use crate::systems::effect::{self, EffectCommand, TimedEffects};
use crate::systems::item::Reservation;
use crate::systems::movement::{MoveTarget, Path, Position, forget, position};
use crate::systems::player::Players;
use crate::systems::stat::{self, AttackSpeedStat, StatSet};

const FILE: &str = "npc_table.json";
const SPAWN_FILE: &str = "spawn_table.json";

const NPC_RESPAWN_DELAY: Seconds = Seconds(5.0);
const RNG_SEED: u64 = 0x1234_5678_9abc_def0;

pub fn register(app: &mut App) {
    effect::source(app, chase);
}

pub fn chase(world: &World, entity: Entity) -> Vec<EffectCommand> {
    if world.get::<Npc>(entity).is_some() && world.get::<AttackTarget>(entity).is_some() {
        static CHASE: OnceLock<EffectCommand> = OnceLock::new();
        vec![CHASE.get_or_init(|| effect::command(&Chasing, &())).clone()]
    } else {
        Vec::new()
    }
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Npc {
    pub def: Id<NpcDef>,
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
    #[serde(deserialize_with = "deserialize_ai")]
    pub ai: &'static dyn Ai,
    pub stats: StatSet,
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
    spawn(world, def_index, at, area.id, group);
}

pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Tiles>, area: Id<AreaDef>) -> Entity {
    let entity = world.spawn(character(def, at, area)).id();
    def.stats.apply(world, entity);
    entity
}

pub fn spawn(
    world: &mut World,
    def_index: Id<NpcDef>,
    at: Pos<Tiles>,
    area: Id<AreaDef>,
    group: u32,
) -> Entity {
    let entity = spawn_actor(world, def_index.get(), at, area);
    world.entity_mut(entity).insert(Npc {
        def: def_index,
        group,
    });
    entity
}

fn character(def: &NpcDef, at: Pos<Tiles>, area: Id<AreaDef>) -> Character {
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
            attack_rate: PlaybackRate(def.stats.get(AttackSpeedStat.into())),
        },
        hitbox: Hitbox {
            size: def.model.get().hitbox(),
        },
        area: AreaTag { area },
    }
}

pub trait Ai: Send + Sync {
    fn name(&self) -> &str;
    fn wanders(&self, rng: &mut Rng) -> bool;
    fn target(&self, hunt: &Hunt) -> Option<Entity>;
}

inventory::collect!(&'static dyn Ai);

fn deserialize_ai<'de, D: Deserializer<'de>>(deserializer: D) -> Result<&'static dyn Ai, D::Error> {
    let name = String::deserialize(deserializer)?;
    inventory::iter::<&'static dyn Ai>()
        .copied()
        .find(|ai| ai.name() == name)
        .ok_or_else(|| serde::de::Error::custom(format!("unknown ai '{name}'")))
}

pub struct Hunt<'a> {
    pub world: &'a World,
    pub players: &'a [Entity],
    pub by_group: &'a HashMap<u32, Vec<Entity>>,
    pub id: Entity,
    pub group: u32,
    pub at: Pos<Tiles>,
    pub area: Id<AreaDef>,
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
        let Some(npc) = world.get::<Npc>(id).cloned() else {
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

fn idle_wander(world: &mut World, rng: &mut Rng, id: Entity, def: &NpcDef, area: Id<AreaDef>) {
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

fn in_aggro(
    world: &World,
    target: Entity,
    at: Pos<Tiles>,
    area: Id<AreaDef>,
    aggro: Tiles,
) -> bool {
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
        let at = random_walkable(&mut rng, region.id).unwrap_or(region.spawn);
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

fn random_walkable(rng: &mut Rng, area_id: Id<AreaDef>) -> Option<Pos<Tiles>> {
    let nodes = &area::areas()[area_id.index()].walkable_nodes;
    if nodes.is_empty() {
        return None;
    }
    Some(nodes[(rng.roll() % nodes.len() as u64) as usize])
}
