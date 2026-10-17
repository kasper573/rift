pub mod body;
pub mod summon;

pub use summon::{Despawn, SetAttitude, ShownFor, SpawnNear, SpawnNpcs, Summoned, dismiss, summon};

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::Replicated;
use bevy_time::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::babble::{BabbleId, Babbler};
use crate::core::content::{
    Content, ContentId, ContentRow, ModuleRule, ModuleSet, RowCheck, serial,
};
use crate::core::math::{Direction, Percent, Pos, Rect, Rgba, Rng};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::systems::actor::model::ModelId;
use crate::systems::actor::{self, Action, Actor, ActorModel, Hitbox, Tint};
use crate::systems::area::{self, AreaId, AreaModule, AreaTag, Wild};
use crate::systems::combat::{Attitude, Dead, Died};
use crate::systems::dialogue;
use crate::systems::interact::Interactive;
use crate::systems::item::{ItemId, Reservation};
use crate::systems::movement::{Position, position};
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::query::TickRoster;
use crate::systems::script::{
    EventKind, Owner, OwnerKind, OwnerRow, Reach, RiftEvent, RiftInstances, RiftScripts,
    ScriptModule, ScriptedRow, area_owner,
};
use crate::systems::skill::{Rank, SkillId};
use crate::systems::stat::{StatMap, Stats};
use crate::systems::status::Statuses;
use crate::systems::visibility::{self, Presence};
use crate::systems::{Character, WorldArea};

pub type NpcId = ContentId<NpcDef>;

const CORPSE_LINGER: Seconds = Seconds(5.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate_once::<Npc>();
}

pub fn can_be_fought(scripts: &RiftScripts, npc: NpcId) -> bool {
    scripts
        .may_do(OwnerRow {
            kind: OwnerKind::Npc,
            row: npc.raw(),
            object: None,
        })
        .reaches
        .contains(&Reach::Hostile)
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Npc {
    pub def: NpcId,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pack(pub u32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Home(pub Pos<Tiles>);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Roams(pub Rect<Tiles>);

#[derive(Component, Default)]
pub struct Near(Vec<Entity>);

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct NpcDef {
    pub name: String,
    pub model: ModelId,
    pub stats: StatMap,
    #[schemars(extend("x-usage" = "the skill a click on a target uses"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_attack: Option<SkillId>,
    #[schemars(extend("x-usage" = "known at these ranks, for the scripts that will cast them"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<NpcSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<NpcModule>,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NpcSkill {
    pub skill: SkillId,
    #[schemars(extend("default" = 1))]
    #[serde(default = "first_rank", skip_serializing_if = "Rank::is_first")]
    pub rank: Rank,
}

fn first_rank() -> Rank {
    Rank::FIRST
}

impl ContentRow for NpcDef {
    const TABLE: &'static str = "npc";

    fn check(&self, check: &mut RowCheck) {
        let mut seen = std::collections::BTreeSet::new();
        for (index, known) in self.skills.iter().enumerate() {
            if !seen.insert(known.skill) {
                check.error(
                    format!("/skills/{index}/skill"),
                    "skill twice",
                    known.skill.raw(),
                    "each skill listed once, at one rank",
                );
            }
            if known.rank == Rank::NONE {
                check.error(
                    format!("/skills/{index}/rank"),
                    "out of range",
                    0,
                    "1 or more",
                );
            }
        }
    }
}

impl ModuleSet for NpcDef {
    type Module = NpcModule;

    fn modules(&self) -> &[NpcModule] {
        &self.modules
    }

    fn rule(module: &NpcModule) -> ModuleRule {
        match module {
            NpcModule::Script(_) => ModuleRule::AnyNumber,
            _ => ModuleRule::AtMostOne,
        }
    }
}

impl ScriptedRow for NpcDef {
    const OWNER: OwnerKind = OwnerKind::Npc;

    fn script(module: &NpcModule) -> Option<&ScriptModule> {
        match module {
            NpcModule::Script(script) => Some(script),
            _ => None,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NpcModule {
    Nameplate(Nameplate),
    Tint(Tint),
    Voice(Voice),
    Respawn(Respawn),
    Loot(Loot),
    #[serde(untagged)]
    Script(ScriptModule),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Nameplate {
    pub role: String,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct Voice {
    pub babble: BabbleId,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct Respawn {
    pub after: Seconds,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Loot {
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<LootItem>,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct LootItem {
    pub item: ItemId,
    #[schemars(extend("x-count" = true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub amount: u32,
    #[schemars(extend("x-blank" = "always"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chance: Option<Percent>,
}

impl NpcDef {
    pub fn nameplate(&self) -> Option<&str> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Nameplate(nameplate) => Some(nameplate.role.as_str()),
            _ => None,
        })
    }

    pub fn tint(&self) -> Rgba {
        self.modules
            .iter()
            .find_map(|module| match module {
                NpcModule::Tint(tint) => Some(tint.rgba),
                _ => None,
            })
            .unwrap_or(Rgba::WHITE)
    }

    pub fn voice(&self) -> Option<BabbleId> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Voice(voice) => Some(voice.babble),
            _ => None,
        })
    }

    pub fn respawn(&self) -> Option<Seconds> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Respawn(respawn) => Some(respawn.after),
            _ => None,
        })
    }

    pub fn loot(&self) -> Option<&Loot> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Loot(loot) => Some(loot),
            _ => None,
        })
    }
}

#[derive(Resource, Default)]
struct NextPack(u32);

pub fn next_pack(world: &mut World) -> Pack {
    let mut next = world.get_resource_or_init::<NextPack>();
    next.0 += 1;
    Pack(next.0)
}

pub fn spawn_all(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    for module in area::modules(&content, area_id) {
        match module {
            AreaModule::Population(population) => {
                let pack = next_pack(world);
                for _ in 0..population.count {
                    spawn_wild(
                        world,
                        &content,
                        area_id,
                        population.npc,
                        population.roams,
                        pack,
                    );
                }
            }
            AreaModule::Resident(resident) => {
                let pack = next_pack(world);
                let entity = spawn(world, resident.npc, resident.at, area_id, pack);
                world.entity_mut(entity).insert(Home(resident.at));
                if let Some(shown) = &resident.shown {
                    let shown = CompiledCondition::new(&content, shown);
                    world.entity_mut(entity).insert(Presence::When(shown));
                }
            }
            _ => {}
        }
    }
}

fn spawn_wild(
    world: &mut World,
    content: &Content,
    area_id: AreaId,
    npc: NpcId,
    roams: Option<Rect<Tiles>>,
    pack: Pack,
) {
    let area = area::load(content, area_id);
    let at = {
        let mut rng = world.resource_mut::<Rng>();
        match roams {
            Some(bounds) => area.wild_grid.random_node_within(&mut rng, bounds),
            None => area.wild_grid.random_node(&mut rng),
        }
    }
    .expect("validated at startup: populations have ground outside the safe zones");
    let entity = spawn(world, npc, at, area_id, pack);
    world.entity_mut(entity).insert(Wild);
    if let Some(bounds) = roams {
        world.entity_mut(entity).insert(Roams(bounds));
    }
}

pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Tiles>, area: AreaId) -> Entity {
    let content = world.resource::<Content>().clone();
    let entity = world.spawn(character(&content, def, at, area)).id();
    world
        .entity_mut(entity)
        .insert(Stats(def.stats.listed().collect()));
    entity
}

pub fn spawn(world: &mut World, def: NpcId, at: Pos<Tiles>, area: AreaId, pack: Pack) -> Entity {
    let content = world.resource::<Content>().clone();
    let entity = spawn_actor(world, def.get(&content), at, area);
    world.entity_mut(entity).insert((
        Npc { def },
        pack,
        Attitude::Friendly,
        body::Body::default(),
        actor::Name {
            name: def.get(&content).name.to_owned(),
        },
    ));
    if let Some(babble) = def.get(&content).voice() {
        world.entity_mut(entity).insert(Babbler(babble));
    }
    start_scripts(world, entity, def);
    entity
}

pub fn despawn(world: &mut World, npc: Entity) {
    world
        .resource_mut::<RiftInstances>()
        .remove_owner(Owner::Entity(OwnerKind::Npc, npc));
    world.despawn(npc);
}

pub fn attacked(world: &mut World, target: Entity, attacker: Entity) {
    if world.get::<Npc>(target).is_none() {
        return;
    }
    world.resource_mut::<RiftInstances>().raise(
        Owner::Entity(OwnerKind::Npc, target),
        EventKind::Attacked,
        Some(attacker),
        Vec::new(),
        |_| true,
    );
}

pub fn defeated(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let deaths: Vec<Died> = deaths
        .read(world.resource::<Messages<Died>>())
        .cloned()
        .collect();
    for died in deaths {
        let is_npc = world.get::<Npc>(died.entity).is_some();
        let owner = area_owner(world);
        world.resource_mut::<RiftInstances>().raise(
            owner,
            EventKind::Died,
            Some(died.entity),
            Vec::new(),
            move |event| match event {
                RiftEvent::Died { by } => by.admits(is_npc),
                _ => false,
            },
        );
        if !is_npc {
            continue;
        }
        let credited = crate::systems::item::credited(world, died.entity, now);
        world.resource_mut::<RiftInstances>().raise(
            Owner::Entity(OwnerKind::Npc, died.entity),
            EventKind::Defeated,
            credited,
            Vec::new(),
            |_| true,
        );
    }
}

struct Watcher {
    npc: Entity,
    at: Pos<Tiles>,
    area: AreaId,
    reach: Tiles,
}

pub fn approach(world: &mut World) {
    let Some(roster) = world.get_resource::<TickRoster>() else {
        return;
    };
    let instances = world.resource::<RiftInstances>();
    let watchers: Vec<Watcher> = instances
        .owners_answering(EventKind::CameNear)
        .iter()
        .filter_map(|&owner| {
            let Owner::Entity(OwnerKind::Npc, npc) = owner else {
                return None;
            };
            let listed = roster.iter().find(|listed| listed.entity == npc)?;
            if listed.dead {
                return None;
            }
            let reach = instances
                .entries(owner, EventKind::CameNear)
                .filter_map(|event| match event {
                    RiftEvent::CameNear { within, .. } => Some(*within),
                    _ => None,
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .unwrap_or(Tiles(0.0));
            Some(Watcher {
                npc,
                at: listed.at,
                area: listed.area,
                reach,
            })
        })
        .collect();
    if watchers.is_empty() {
        return;
    }
    let approached: Vec<(Watcher, Vec<Entity>)> = watchers
        .into_iter()
        .map(|watcher| {
            let mut now: Vec<Entity> = roster
                .iter()
                .filter(|listed| {
                    listed.entity != watcher.npc
                        && listed.area == watcher.area
                        && !listed.dead
                        && listed.at.distance(watcher.at) <= watcher.reach
                        && visibility::coexist(world, watcher.npc, listed.entity)
                        && !dialogue::in_conversation(world, listed.entity)
                })
                .map(|listed| listed.entity)
                .collect();
            now.sort_unstable();
            (watcher, now)
        })
        .collect();
    for (Watcher { npc, at, .. }, now) in approached {
        let before: Vec<Entity> = world
            .get::<Near>(npc)
            .map(|near| near.0.clone())
            .unwrap_or_default();
        if now == before {
            continue;
        }
        for &other in now.iter().filter(|other| !before.contains(other)) {
            let is_npc = world.get::<Npc>(other).is_some();
            let def = world.get::<Npc>(other).map(|npc| npc.def);
            let distance =
                position(world, other).map_or(Tiles(f32::MAX), |there| there.distance(at));
            world.resource_mut::<RiftInstances>().raise(
                Owner::Entity(OwnerKind::Npc, npc),
                EventKind::CameNear,
                Some(other),
                Vec::new(),
                move |event| match event {
                    RiftEvent::CameNear { within, by, npc } => {
                        distance <= *within
                            && by.admits(is_npc)
                            && npc.is_none_or(|npc| Some(npc) == def)
                    }
                    _ => false,
                },
            );
        }
        world.entity_mut(npc).insert(Near(now));
    }
}

fn character(content: &Content, def: &NpcDef, at: Pos<Tiles>, area: AreaId) -> Character {
    Character {
        replicated: Replicated,
        position: Position::at(at),
        actor: Actor {
            color: def.tint(),
            dir: Direction::S,
            action: Action::Idle,
            model: def.model,
            attack_rate: PlaybackRate::default(),
        },
        hitbox: Hitbox {
            size: ActorModel::get(content, def.model).hitbox(),
        },
        area: AreaTag { area },
    }
}

fn start_scripts(world: &mut World, entity: Entity, def: NpcId) {
    let procedures = world
        .resource::<RiftScripts>()
        .of(OwnerRow {
            kind: OwnerKind::Npc,
            row: def.raw(),
            object: None,
        })
        .to_vec();
    let owner = Owner::Entity(OwnerKind::Npc, entity);
    let interactive = procedures
        .iter()
        .flat_map(|procedure| procedure.entry(EventKind::Interacted))
        .find_map(|(_, event)| match event {
            RiftEvent::Interacted { verb, reach, .. } => Some(Interactive {
                verb: *verb,
                reach: *reach,
            }),
            _ => None,
        });
    match interactive {
        Some(interactive) => {
            world.entity_mut(entity).insert(interactive);
        }
        None => {
            world.entity_mut(entity).remove::<Interactive>();
        }
    }
    let mut instances = world.resource_mut::<RiftInstances>();
    instances.spawn_owner(owner, procedures);
    instances.raise(owner, EventKind::Started, None, Vec::new(), |_| true);
}

type NpcIds = QueryState<Entity, With<Npc>>;

pub fn run_respawn(world: &mut World, npcs: &mut NpcIds) {
    let content = world.resource::<Content>().clone();
    let time = Seconds(world.resource::<Time>().elapsed_secs());
    let ids: Vec<Entity> = npcs.iter(world).collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for id in ids {
            let Some(since) = world.get::<Dead>(id).map(|dead| dead.at) else {
                continue;
            };
            world
                .resource_mut::<RiftInstances>()
                .remove_owner(Owner::Entity(OwnerKind::Npc, id));
            let Some(npc) = world.get::<Npc>(id).map(|npc| npc.def) else {
                continue;
            };
            let def = npc.get(&content);
            let Some(delay) = def.respawn() else {
                if time - since >= CORPSE_LINGER {
                    world.despawn(id);
                }
                continue;
            };
            if time - since < delay {
                continue;
            }
            let Some(region) = area::of(world, id) else {
                continue;
            };
            let grid = region.grid_for(area::wild(world, id));
            let at = match (world.get::<Home>(id), world.get::<Roams>(id)) {
                (Some(&Home(home)), _) => Some(home),
                (None, Some(&Roams(bounds))) => grid.random_node_within(&mut rng, bounds),
                (None, None) => grid.random_node(&mut rng),
            }
            .unwrap_or_else(|| region.fallback_spawn());
            crate::systems::combat::revive(world, id, at);
            world
                .entity_mut(id)
                .insert((Attitude::Friendly, body::Body::default(), Near::default()))
                .remove::<Reservation>()
                .insert(Statuses::default());
            start_scripts(world, id, npc);
        }
    });
}
