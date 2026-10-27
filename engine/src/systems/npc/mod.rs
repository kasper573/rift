pub mod body;
pub mod summon;

pub use summon::{Despawn, SetAttitude, ShownFor, SpawnNear, SpawnNpcs, Summoned, dismiss, summon};

use bevy_app::App;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_time::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::babble::{BabbleId, Babbler};
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck, serial,
};
use crate::core::geometry::Radians;
use crate::core::math::{Percent, Pos, Rect, Rgba, Rng};
use crate::core::time::{PlaybackRate, Seconds};
use crate::core::units::{MeterPos, Meters, MetersPerSec};
use crate::systems::actor::model::ModelId;
use crate::systems::actor::wearing::{self, Wearing};
use crate::systems::actor::{self, Action, Actor, ActorModel, Hitbox, Pose, Tint};
use crate::systems::area::{self, AreaId, AreaModule, AreaTag, Wild};
use crate::systems::combat::{Attitude, Dead, Died};
use crate::systems::interact::Interactive;
use crate::systems::item::{ItemId, Reservation, Wear};
use crate::systems::movement::{self, NavMesh, Position, Spots, position};
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::query::TickRoster;
use crate::systems::script::{
    EventKind, Owner, OwnerRow, Reach, RiftEvent, RiftInstances, RiftScripts, RunnerKind,
    ScriptModule, ScriptedRow, area_owner,
};
use crate::systems::skill::{self, Rank, SkillId};
use crate::systems::stat::Stats;
use crate::systems::status::Statuses;
use crate::systems::visibility::{self, Presence};
use crate::systems::{Character, WorldArea, dialogue};

pub type NpcId = ContentId<NpcDef>;

const CORPSE_LINGER: Seconds = Seconds(5.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate_once::<Npc>();
}

/// The ranks an npc holds its skills at lie within those skills' ranks.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<NpcDef>(&content) {
        let mut check = RowCheck::default();
        for (index, listed) in def.skills.iter().enumerate() {
            skill::check_rank(
                &mut check,
                format!("/skills/{index}/rank"),
                &content,
                listed.skill,
                listed.rank,
            );
        }
        check_wearing(&content, loading, id, def, &mut check);
        loading.row_errors(id, check);
    }
}

/// A wear list holds what hands show, asking no more hands than the body has; one the body cannot
/// show at all loads, and says so.
fn check_wearing(
    content: &Content,
    loading: &mut Loading,
    id: NpcId,
    def: &NpcDef,
    check: &mut RowCheck,
) {
    for (index, &item) in def.wearing.iter().enumerate() {
        if !matches!(item.get(content).wear(), Some(Wear::Hand { .. })) {
            check.error(
                format!("/wearing/{index}"),
                "not held",
                format!("wearing lists {}, which no hand holds", item.name(content)),
                "items a hand holds, each naming its stance",
            );
        }
    }
    let Some(model) = ActorModel::loaded(content, def.model) else {
        return;
    };
    let hands = model.hands().len();
    let skeleton = model.skeleton_id();
    if hands == 0 {
        for (index, &item) in def.wearing.iter().enumerate() {
            loading.row_warning(
                id,
                &format!("/wearing/{index}"),
                ContentProblem::new(
                    "never shown",
                    format!(
                        "wearing lists {}, but {}.skeleton has no hands: listed, never shown",
                        item.name(content),
                        skeleton.name(content)
                    ),
                    "a body with hands, for the item to show",
                ),
            );
        }
        return;
    }
    let asked = wearing::asked(content, &def.wearing);
    if asked > hands {
        let names: Vec<&str> = def.wearing.iter().map(|item| item.name(content)).collect();
        check.error(
            "/wearing",
            "too many hands",
            format!(
                "wearing asks {asked} hands ({}) of a body with {hands}",
                names.join(", ")
            ),
            format!(
                "at most {hands} hands, as {}.skeleton has",
                skeleton.name(content)
            ),
        );
    }
}

pub fn can_be_fought(scripts: &RiftScripts, npc: NpcId) -> bool {
    scripts
        .may_do(OwnerRow::of_row(npc))
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
pub struct Home(pub Pos<Meters>);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Roams(pub Rect<Meters>);

#[derive(Component, Default)]
pub struct Near(Vec<Entity>);

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct NpcDef {
    pub name: String,
    pub model: ModelId,
    pub stats: Stats,
    #[schemars(extend("x-usage" = "the skill a click on a target uses"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_attack: Option<SkillId>,
    #[schemars(extend("x-usage" = "known at these ranks, for the scripts that will cast them"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<NpcSkill>,
    /// What its hands show, in their order: looks alone, granting no skill, stat or drop.
    #[schemars(extend("x-usage" = "shown in its hands"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wearing: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<NpcModule>,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NpcSkill {
    pub skill: SkillId,
    #[schemars(extend("default" = 1))]
    #[serde(default = "Rank::first", skip_serializing_if = "Rank::is_first")]
    pub rank: Rank,
}

impl ContentFile for NpcDef {
    const TABLE: &'static str = "npc";

    fn fresh(near: &Nearby) -> Result<NpcDef, String> {
        Ok(NpcDef {
            name: near.name(),
            model: near.file()?,
            stats: Stats::default(),
            basic_attack: None,
            skills: Vec::new(),
            wearing: Vec::new(),
            modules: Vec::new(),
        })
    }

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

impl NamedContent for NpcDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for NpcDef {
    type Module = NpcModule;

    fn modules(&self) -> &[NpcModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<NpcModule> {
        &mut self.modules
    }

    fn rule(module: &NpcModule) -> ModuleRule {
        match module {
            NpcModule::Script(_) => ModuleRule::AnyNumber,
            _ => ModuleRule::AtMostOne,
        }
    }
}

impl ScriptedRow for NpcDef {
    const OWNER: RunnerKind = RunnerKind::Npc;

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
                let mesh = &area::load(&content, area_id).mesh;
                let at = movement::place_beside(world, area_id, mesh, resident.at);
                let entity = spawn(world, resident.npc, at, area_id, pack);
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
    roams: Option<Rect<Meters>>,
    pack: Pack,
) {
    let mesh = &area::load(content, area_id).wild_mesh;
    let at = world
        .resource_scope(|world, mut rng: Mut<Rng>| {
            let spots = world.resource::<Spots>();
            roaming_spot(mesh, &mut rng, spots, area_id, roams)
        })
        .expect("validated at startup: populations have ground outside the safe zones");
    let entity = spawn(world, npc, at, area_id, pack);
    match roams {
        Some(bounds) => world.entity_mut(entity).insert((Wild, Roams(bounds))),
        None => world.entity_mut(entity).insert(Wild),
    };
}

/// A bare actor of the definition, with none of an NPC's scripts or standing; `spawn` makes a
/// whole NPC of it.
pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Meters>, area: AreaId) -> Entity {
    let content = world.resource::<Content>().clone();
    world.spawn(actor_bundle(&content, def, at, area)).id()
}

pub fn spawn(world: &mut World, def: NpcId, at: Pos<Meters>, area: AreaId, pack: Pack) -> Entity {
    let content = world.resource::<Content>().clone();
    let row = def.get(&content);
    let mut entity = world.spawn((
        actor_bundle(&content, row, at, area),
        Npc { def },
        pack,
        Attitude::Friendly,
        body::Body::default(),
        actor::Name {
            name: row.name.to_owned(),
        },
    ));
    if let Some(babble) = row.voice() {
        entity.insert(Babbler(babble));
    }
    let entity = entity.id();
    start_scripts(world, entity, def);
    entity
}

pub fn despawn(world: &mut World, npc: Entity) {
    world
        .resource_mut::<RiftInstances>()
        .remove_owner(Owner::Entity(RunnerKind::Npc, npc));
    world.despawn(npc);
}

pub fn attacked(world: &mut World, target: Entity, attacker: Entity) {
    if world.get::<Npc>(target).is_none() {
        return;
    }
    world.resource_mut::<RiftInstances>().raise(
        Owner::Entity(RunnerKind::Npc, target),
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
            Owner::Entity(RunnerKind::Npc, died.entity),
            EventKind::Defeated,
            credited,
            Vec::new(),
            |_| true,
        );
    }
}

struct Watcher {
    npc: Entity,
    at: Pos<Meters>,
    area: AreaId,
    reach: Meters,
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
            let Owner::Entity(RunnerKind::Npc, npc) = owner else {
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
                .unwrap_or(Meters(0.0));
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
                position(world, other).map_or(Meters(f32::MAX), |there| there.distance(at));
            world.resource_mut::<RiftInstances>().raise(
                Owner::Entity(RunnerKind::Npc, npc),
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

fn actor_bundle(content: &Content, def: &NpcDef, at: Pos<Meters>, area: AreaId) -> impl Bundle {
    let model = ActorModel::get(content, def.model);
    let shown = Wearing {
        hands: wearing::listed(content, model.hands().len(), &def.wearing),
    };
    let character = Character::new(
        Position::at(at),
        Actor {
            color: def.tint(),
            facing: Radians::SOUTH,
            action: Action::Idle,
            pose: Pose::Idle,
            model: def.model,
            attack_rate: PlaybackRate::default(),
            speed: MetersPerSec::default(),
        },
        Hitbox(model.hitbox()),
        AreaTag { area },
    );
    (character, def.stats.clone(), shown)
}

fn start_scripts(world: &mut World, entity: Entity, def: NpcId) {
    let procedures = world
        .resource::<RiftScripts>()
        .of(OwnerRow::of_row(def))
        .to_vec();
    let owner = Owner::Entity(RunnerKind::Npc, entity);
    let interactive = Interactive::of(&procedures);
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

/// A walkable spot within `roams`, or anywhere without it, that nobody stands on if there is one.
fn roaming_spot(
    mesh: &NavMesh,
    rng: &mut Rng,
    spots: &Spots,
    area: AreaId,
    roams: Option<Rect<Meters>>,
) -> Option<Pos<Meters>> {
    let mut draw = |fits: &dyn Fn(Pos<Meters>) -> bool| match roams {
        Some(bounds) => mesh.random_point_within(rng, bounds, fits),
        None => mesh.random_point(rng, fits),
    };
    draw(&|at| spots.crowding(area, at, None) == 0).or_else(|| draw(&|_| true))
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
                .remove_owner(Owner::Entity(RunnerKind::Npc, id));
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
            let (Some(region), Some(&AreaTag { area: area_id })) =
                (area::of(world, id), world.get::<AreaTag>(id))
            else {
                continue;
            };
            let mesh = region.mesh_for(area::wild(world, id), false);
            let roams = world.get::<Roams>(id).map(|roams| roams.0);
            let spots = world.resource::<Spots>();
            let at = match world.get::<Home>(id) {
                Some(&Home(home)) => Some(home),
                None => roaming_spot(mesh, &mut rng, spots, area_id, roams),
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
