pub mod body;
pub mod summon;

pub use summon::{Despawn, SetAttitude, ShownFor, SpawnNear, SpawnNpcs, Summoned, dismiss, summon};

use bevy_app::App;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;
use bevy_replication::Replicated;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::babble::{BabbleId, Babbler};
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck, SchemaMark,
};
use crate::core::math::{Pos, Rect};
use crate::core::tick::Planner;
use crate::core::time;
use crate::core::units::{MeterPos, Meters, MetersPerSec, PlaybackRate, Radians, Seconds};
use crate::systems::GameSettings;
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::{
    self, Action, Actor, ActorModel, Appearance, Hitbox, Pose, appearance,
};
use crate::systems::area::{Area, AreaId, AreaTag, Resident};
use crate::systems::combat::{Attitude, Dead, Died};
use crate::systems::identity::{GlobalId, Names};
use crate::systems::interact::Interactive;
use crate::systems::item::{DropRow, ItemId, Wear};
use crate::systems::movement::{self, Position, Spot, position};
use crate::systems::script::condition::CompiledCondition;
use crate::systems::script::query::{Among, TickRoster};
use crate::systems::script::{
    EffectReach, EventKind, Owner, OwnerRow, RiftEvent, RiftInstances, RiftScripts, RunnerKind,
    ScriptModule, ScriptedRow, area_owner,
};
use crate::systems::skill::{self, Rank, SkillId};
use crate::systems::stat::Stats;
use crate::systems::visibility::{self, Presence};
use crate::systems::{dialogue, stat};

pub type NpcId = ContentId<NpcDef>;

pub fn register(app: &mut App) {
    use bevy_replication::AppReplicationExt;
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
        if ActorModel::loaded(&content, def.appearance.model).is_some() {
            appearance::check(&content, &def.appearance, "/appearance", &mut check);
        }
        check_held(&content, loading, id, def, &mut check);
        loading.row_errors(id, check);
    }
}

/// What an npc holds is what hands show, all of it shown together as equipping would place it; a
/// list the body cannot show at all loads, and says so.
fn check_held(
    content: &Content,
    loading: &mut Loading,
    id: NpcId,
    def: &NpcDef,
    check: &mut RowCheck,
) {
    for (index, &item) in def.held.iter().enumerate() {
        if item.get(content).wear().and_then(Wear::grasp).is_none() {
            check.error(
                format!("/held/{index}"),
                "not held",
                format!("held lists {}, which no hand holds", item.name(content)),
                "items a hand holds, each naming its stance",
            );
        }
    }
    let Some(model) = ActorModel::loaded(content, def.appearance.model) else {
        return;
    };
    let hands = model.hands().len();
    let skeleton = model.skeleton_id();
    if hands == 0 {
        for (index, &item) in def.held.iter().enumerate() {
            loading.row_warning(
                id,
                &format!("/held/{index}"),
                ContentProblem::new(
                    "never shown",
                    format!(
                        "held lists {}, but {}.skeleton has no hands: listed, never shown",
                        item.name(content),
                        skeleton.name(content)
                    ),
                    "a body with hands, for the item to show",
                ),
            );
        }
        return;
    }
    let left_out = held::listed(content, hands, &def.held).returned;
    if !left_out.is_empty() {
        let names = |items: &[ItemId]| {
            ui::words::join_and(
                &items
                    .iter()
                    .map(|item| item.name(content).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        check.error(
            "/held",
            "left out",
            format!(
                "held lists {}, but placed as equipping places them in the {hands} hands of {}, {} \
                 is put back",
                names(&def.held),
                skeleton.name(content),
                names(&left_out),
            ),
            "items the hands show together, each where its stance puts it",
        );
    }
}

pub fn can_be_fought(scripts: &RiftScripts, npc: NpcId) -> bool {
    scripts
        .may_do(OwnerRow::of_row(npc))
        .reaches
        .contains(&EffectReach::Hostile)
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Npc {
    pub def: NpcId,
}

/// The npcs spawned together, named from their world's pool so no two packs of an area are alike.
/// The world's [`Packs`] follows it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[component(immutable, on_insert = file_pack, on_remove = unfile_pack)]
pub struct Pack(pub GlobalId);

/// Every npc with a pack, by its pack.
#[derive(Resource, Default)]
pub struct Packs(HashMap<Pack, Vec<Entity>>);

impl Packs {
    pub fn members(&self, pack: Pack) -> &[Entity] {
        self.0.get(&pack).map_or(&[], Vec::as_slice)
    }
}

fn file_pack(mut world: DeferredWorld, context: HookContext) {
    let pack = *world
        .get::<Pack>(context.entity)
        .expect("the component being inserted");
    packs(&mut world)
        .0
        .entry(pack)
        .or_default()
        .push(context.entity);
}

fn unfile_pack(mut world: DeferredWorld, context: HookContext) {
    let pack = *world
        .get::<Pack>(context.entity)
        .expect("the component being removed");
    let mut packs = packs(&mut world);
    let Some(members) = packs.0.get_mut(&pack) else {
        return;
    };
    members.retain(|&member| member != context.entity);
    if members.is_empty() {
        packs.0.remove(&pack);
    }
}

fn packs<'w>(world: &'w mut DeferredWorld) -> Mut<'w, Packs> {
    world
        .get_resource_mut::<Packs>()
        .expect("a world spawning npcs holds its packs")
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Home(pub Pos<Meters>);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Roams(pub Rect<Meters>);

#[derive(Component, Default)]
pub struct Near(Vec<Entity>);

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct NpcDef {
    pub name: String,
    pub appearance: Appearance,
    pub stats: Stats,
    #[schemars(transform = SchemaMark::Usage.with("the skill a click on a target uses"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_attack: Option<SkillId>,
    #[schemars(transform = SchemaMark::Usage.with("known at these ranks, for the scripts that will cast them"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<NpcSkill>,
    /// What its hands show, each placed in turn as equipping places it: looks alone, granting no
    /// skill, stat or drop.
    #[schemars(transform = SchemaMark::Usage.with("shown in its hands"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<NpcModule>,
    /// How far from home it wanders, when a wander of its script names no leash of its own.
    pub leash: Meters,
    /// How it stands toward the players as it comes to life, until a script sets it otherwise.
    pub attitude: Attitude,
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
            appearance: Appearance::of(near.file()?),
            stats: Stats::default(),
            basic_attack: None,
            skills: Vec::new(),
            held: Vec::new(),
            modules: Vec::new(),
            leash: Meters(0.0),
            attitude: Attitude::Friendly,
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
    Voice(Voice),
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

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Loot {
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DropRow>,
}

impl NpcDef {
    pub fn nameplate(&self) -> Option<&str> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Nameplate(nameplate) => Some(nameplate.role.as_str()),
            _ => None,
        })
    }

    pub fn voice(&self) -> Option<BabbleId> {
        self.modules.iter().find_map(|module| match module {
            NpcModule::Voice(voice) => Some(voice.babble),
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

pub fn next_pack(world: &mut World) -> Pack {
    Pack(world.resource_mut::<Names>().give())
}

/// Spawns a resident at its spot in the area, facing and posed as it lives there.
pub fn settle(
    world: &mut World,
    content: &Content,
    area: &Area,
    area_id: AreaId,
    resident: &Resident,
    pack: Pack,
) -> Entity {
    let at = movement::place_beside(world, area_id, &area.mesh(), resident.at);
    let entity = spawn(world, resident.npc, at, area_id, Some(pack));
    world.entity_mut(entity).insert(Home(resident.at));
    if let Some(mut actor) = world.get_mut::<Actor>(entity) {
        actor.facing = resident.facing.radians();
        actor.pose = resident.pose;
    }
    if let Some(shown) = &resident.shown {
        let shown = CompiledCondition::new(content, shown);
        world.entity_mut(entity).insert(Presence::When(shown));
    }
    entity
}

/// A bare actor of the definition, with none of an NPC's scripts or standing; `spawn` makes a
/// whole NPC of it.
pub fn spawn_actor(world: &mut World, def: &NpcDef, at: Pos<Meters>, area: AreaId) -> Entity {
    let content = world.resource::<Content>().clone();
    world.spawn(actor_bundle(&content, def, at, area)).id()
}

/// Spawns an npc of `def` at `at`, in `pack` or one of its own.
pub fn spawn(
    world: &mut World,
    def: NpcId,
    at: Pos<Meters>,
    area: AreaId,
    pack: Option<Pack>,
) -> Entity {
    let pack = pack.unwrap_or_else(|| next_pack(world));
    let content = world.resource::<Content>().clone();
    let row = def.get(&content);
    let mut entity_mut = world.spawn((
        actor_bundle(&content, row, at, area),
        Npc { def },
        pack,
        row.attitude,
        body::Body::default(),
        actor::Name {
            name: row.name.to_owned(),
        },
    ));
    if let Some(babble) = row.voice() {
        entity_mut.insert(Babbler(babble));
    }
    let entity = entity_mut.id();
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
    let now = time::uptime(world);
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

/// Tells every npc watching for whoever comes near who did: who is near each watcher is found on
/// every core from the tick's roster, then raised in the order the watchers were found.
#[derive(Default)]
pub struct ApproachWatchers {
    approached: Vec<(Watcher, Vec<Entity>)>,
}

impl Planner for ApproachWatchers {
    fn plan(&mut self, world: &World) {
        self.approached.clear();
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
                if stat::is_dead(world, npc) {
                    return None;
                }
                let at = position(world, npc)?;
                let area = world.get::<AreaTag>(npc)?.area;
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
                    at,
                    area,
                    reach,
                })
            })
            .collect();
        self.approached = watchers
            .into_par_iter()
            .map(|watcher| {
                let mut now: Vec<Entity> = roster
                    .near(Among::Anyone, watcher.at, watcher.reach)
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
    }

    fn apply(&mut self, world: &mut World) {
        for (Watcher { npc, at, .. }, now) in self.approached.drain(..) {
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
                let distance = position(world, other).map(|there| there.distance(at));
                world.resource_mut::<RiftInstances>().raise(
                    Owner::Entity(RunnerKind::Npc, npc),
                    EventKind::CameNear,
                    Some(other),
                    Vec::new(),
                    move |event| match event {
                        RiftEvent::CameNear { within, by, npc } => {
                            distance.is_some_and(|distance| distance <= *within)
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
}

fn actor_bundle(content: &Content, def: &NpcDef, at: Pos<Meters>, area: AreaId) -> impl Bundle {
    let model = ActorModel::get(content, def.appearance.model);
    let shown = Held {
        hands: held::listed(content, model.hands().len(), &def.held).hands,
    };
    (
        Replicated,
        Position::at(at),
        Spot { area, at },
        AreaTag { area },
        Actor {
            facing: Radians::SOUTH,
            action: Action::Idle,
            pose: Pose::Idle,
            attack_rate: PlaybackRate::default(),
            speed: MetersPerSec::default(),
        },
        Hitbox(model.hitbox()),
        def.appearance.clone(),
        def.stats.clone(),
        shown,
    )
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

type FallenNpcs = QueryState<(Entity, &'static Dead), With<Npc>>;

/// Ends the scripts of every fallen npc, and takes its body away once it has lain as long as
/// corpses linger.
pub fn clear_fallen(world: &mut World, fallen: &mut FallenNpcs) {
    let time = time::uptime(world);
    let linger = world.resource::<GameSettings>().corpses_linger;
    let ids: Vec<(Entity, Seconds)> = fallen.iter(world).map(|(id, dead)| (id, dead.at)).collect();
    for (id, since) in ids {
        world
            .resource_mut::<RiftInstances>()
            .remove_owner(Owner::Entity(RunnerKind::Npc, id));
        if time - since >= linger {
            world.despawn(id);
        }
    }
}
