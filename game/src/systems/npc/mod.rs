mod aggressive;
mod defensive;
mod observation;
mod pacifist;
mod protective;
mod stands;
mod strolls;
mod summon;

pub use aggressive::Aggressive;
pub use defensive::Defensive;
pub use observation::{Observation, Observed, observe};
pub use pacifist::Pacifist;
pub use protective::Protective;
pub use stands::Stands;
pub use strolls::Strolls;
pub use summon::{ShownFor, SpawnNear, SpawnNpcs, Summoned, TurnHostile, dismiss};

use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::babble::{BabbleId, Babbler};
use crate::core::math::{Direction, Pos, Rng};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::data;
use crate::systems::actor::{self, Action, Actor, Hitbox, Rgba, set_action};
use crate::systems::area::{self, AreaTag, Wild};
use crate::systems::combat::{Attackers, Attitude};
use crate::systems::effect::{self, Effect, TimedEffects};
use crate::systems::interact::{self, Counterpart, Interaction, Interactive};
use crate::systems::item::Reservation;
use crate::systems::movement::{Grid, MoveTarget, Path, Position, position};
use crate::systems::player::Players;
use crate::systems::reach::{self, ReachAct};
use crate::systems::rewards::KillCredited;
use crate::systems::rule::{Encounter, Outcome, Terms};
use crate::systems::stat::{self, Stat, StatKind, Stats};
use crate::systems::visibility::{self, Presence};
use crate::systems::{Character, WorldArea};

const CORPSE_LINGER: Seconds = Seconds(5.0);
const LEASH_TRIES: u32 = 16;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Npc>();
    effect::source(app, chase);
    interact::interaction_source(app, interaction);
    area::lock_warps(app, data::npc::TABLE.iter().flat_map(|def| def.guards));
}

pub fn conversation_starts() -> Vec<data::dialogue::Id> {
    data::npc::TABLE
        .iter()
        .flat_map(|def| {
            def.interaction
                .iter()
                .flat_map(interact::conversation_starts)
        })
        .chain(observation::starts())
        .chain(
            data::npc::TABLE
                .iter()
                .flat_map(|def| def.on_defeat)
                .flat_map(|outcome| outcome.leads_to()),
        )
        .collect()
}

pub fn check(assets: &AssetService) {
    let outcomes = data::npc::TABLE.iter().flat_map(|def| {
        def.interaction
            .iter()
            .flat_map(|interaction| interaction.responses)
            .flat_map(|response| response.then)
            .chain(
                def.observations
                    .iter()
                    .flat_map(|observation| observation.then),
            )
            .chain(def.on_defeat)
    });
    for outcome in outcomes {
        outcome.check(assets);
    }
    for lock in data::npc::TABLE.iter().flat_map(|def| def.guards) {
        lock.check(assets);
    }
    for speaker in crate::systems::notification::speakers() {
        if speaker.get().babble.is_none() {
            panic!("notification speaker {speaker:?} has no babble");
        }
    }
}

pub fn defeated(world: &mut World, mut credits: Local<MessageCursor<KillCredited>>) {
    let credits: Vec<KillCredited> = credits
        .read(world.resource::<Messages<KillCredited>>())
        .copied()
        .collect();
    for credit in credits {
        let terms = Terms {
            requires: &[],
            costs: &[],
            outcomes: credit.npc.get().on_defeat,
        };
        let encounter = Encounter {
            with: Some(credit.victim),
            tether: None,
        };
        terms.settle(world, credit.credited, encounter).ok();
    }
}

fn interaction(world: &World, entity: Entity) -> Option<&'static Interaction> {
    let def = world.get::<Npc>(entity)?.def.get();
    let friendly = world.get::<Attitude>(entity) == Some(&Attitude::Friendly);
    def.interaction.as_ref().filter(|_| friendly)
}

pub fn chase(world: &World, entity: Entity) -> Vec<Effect> {
    if world.get::<Npc>(entity).is_some()
        && reach::intent(world, entity, ReachAct::Attack).is_some()
    {
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Home(pub Pos<Tiles>);

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DeadAt {
    pub at: Seconds,
}

pub struct NpcDef {
    pub display_name: &'static str,
    pub role: Option<&'static str>,
    pub attitude: Attitude,
    pub respawn: Option<Seconds>,
    pub model: data::model::Id,
    pub babble: Option<BabbleId>,
    pub tint: Rgba,
    pub ai: &'static dyn Ai,
    pub stats: &'static [Stat],
    pub aggro: Tiles,
    pub rewards: &'static [crate::systems::rewards::Reward],
    pub interaction: Option<Interaction>,
    pub observations: &'static [Observation],
    pub guards: &'static [area::WarpLock],
    pub on_defeat: &'static [&'static dyn Outcome],
}

#[derive(Resource, Default)]
struct NextPack(u32);

pub fn next_pack(world: &mut World) -> Pack {
    let mut next = world.get_resource_or_init::<NextPack>();
    next.0 += 1;
    Pack(next.0)
}

pub fn spawn_all(world: &mut World) {
    let area_id = world.resource::<crate::systems::WorldArea>().0;
    let assets = world.resource::<AssetService>().clone();
    let def = area_id.get();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for population in def.populations {
            let pack = next_pack(world);
            for _ in 0..population.count {
                spawn_wild(world, &assets, &mut rng, area_id, population.npc, pack);
            }
        }
    });
    let area = assets.resolve(def.map, area::build_area);
    for resident in def.residents {
        let at = area
            .marker(resident.at)
            .expect("validated at startup: residents stand on markers")
            .center();
        let pack = next_pack(world);
        let entity = spawn(world, resident.npc, at, area_id, pack);
        world.entity_mut(entity).insert(Home(at));
        if !resident.shown.is_empty() {
            world
                .entity_mut(entity)
                .insert(Presence::When(resident.shown));
        }
    }
}

fn spawn_wild(
    world: &mut World,
    assets: &AssetService,
    rng: &mut Rng,
    area_id: area::Id,
    def: data::npc::Id,
    pack: Pack,
) {
    let area = assets.resolve(area_id.get().map, area::build_area);
    let at = area
        .wild_grid
        .random_node(rng)
        .expect("validated at startup: populations have ground outside the safe zones");
    let entity = spawn(world, def, at, area_id, pack);
    world.entity_mut(entity).insert(Wild);
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
    world.entity_mut(entity).insert((
        Npc { def },
        Counterpart::Npc(def),
        pack,
        def.get().attitude,
        actor::Name {
            name: def.get().display_name.to_owned(),
        },
    ));
    if def.get().interaction.is_some() {
        world.entity_mut(entity).insert(Interactive);
    }
    if let Some(babble) = def.get().babble {
        world.entity_mut(entity).insert(Babbler(babble));
    }
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
                .resolve(def.model.get().sheet, actor::build_model)
                .hitbox(),
        },
        area: AreaTag { area },
    }
}

pub trait Ai: Send + Sync {
    fn wanders(&self, rng: &mut Rng) -> bool;
    fn target(&self, hunt: &Hunt) -> Option<Entity>;
    fn leash(&self) -> Option<Tiles> {
        None
    }
}

pub struct Hunt<'a> {
    pub world: &'a World,
    pub players: &'a [Entity],
    pub enemies_by_pack: &'a HashMap<Pack, Vec<Entity>>,
    pub id: Entity,
    pub pack: Pack,
    pub at: Pos<Tiles>,
    pub area: area::Id,
    pub shelter: Option<&'a area::Area>,
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
                || !visibility::coexist(self.world, self.id, candidate)
                || !accept(candidate)
            {
                continue;
            }
            if let Some(at) = position(self.world, candidate) {
                let distance = self.at.distance(at);
                if distance <= self.aggro
                    && best.is_none_or(|(_, best)| distance < best)
                    && !self.shelter.is_some_and(|region| region.safe(at))
                {
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
    let region = assets.resolve(world.resource::<WorldArea>().0.get().map, area::build_area);
    let ids: Vec<Entity> = npcs.iter(world).collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for id in ids {
            if stat::is_dead(world, id) {
                reach::forget(world, id);
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
            let wild = area::wild(world, id);
            let shelter = wild.then_some(region);

            if let Some(target) = reach::intent(world, id, ReachAct::Attack) {
                if in_aggro(world, target, at, area, def.aggro)
                    && !sheltered(world, shelter, target)
                {
                    continue;
                }
                reach::forget(world, id);
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
                    shelter,
                    aggro: def.aggro,
                };
                def.ai.target(&hunt)
            };
            if let Some(target) = target {
                reach::intend(world, id, target, ReachAct::Attack);
                continue;
            }
            idle_wander(world, &mut rng, id, def, region.grid_for(wild));
        }
    });
}

fn idle_wander(world: &mut World, rng: &mut Rng, id: Entity, def: &NpcDef, grid: &Grid) {
    if world.get::<MoveTarget>(id).is_some() || world.get::<Path>(id).is_some() {
        return;
    }
    if !def.ai.wanders(rng) {
        return;
    }
    let Some(at) = position(world, id) else {
        return;
    };
    let node = match (def.ai.leash(), world.get::<Home>(id)) {
        (Some(leash), Some(&Home(home))) => (0..LEASH_TRIES)
            .filter_map(|_| grid.random_reachable(rng, at))
            .find(|node| node.distance(home) <= leash),
        _ => grid.random_reachable(rng, at),
    };
    if let Some(node) = node {
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

fn sheltered(world: &World, shelter: Option<&area::Area>, target: Entity) -> bool {
    shelter.is_some_and(|region| position(world, target).is_some_and(|at| region.safe(at)))
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
            let Some(def) = world.get::<Npc>(id).map(|npc| npc.def.get()) else {
                continue;
            };
            let Some(delay) = def.respawn else {
                if time - since >= CORPSE_LINGER {
                    world.entity_mut(id).despawn();
                }
                continue;
            };
            if time - since < delay {
                continue;
            }
            let Some(region) = area::of(world, id) else {
                continue;
            };
            let at = match world.get::<Home>(id) {
                Some(&Home(home)) => home,
                None => region
                    .grid_for(area::wild(world, id))
                    .random_node(&mut rng)
                    .unwrap_or(region.spawn),
            };
            if let Some(mut position) = world.get_mut::<Position>(id) {
                position.pos = at;
            }
            if let Some(mut actor) = world.get_mut::<Actor>(id) {
                set_action(&mut actor, Action::Idle);
            }
            world
                .entity_mut(id)
                .insert(def.attitude)
                .remove::<DeadAt>()
                .remove::<Reservation>()
                .remove::<TimedEffects>();
            stat::refill(world, id);
            reach::forget(world, id);
        }
    });
}
