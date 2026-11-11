pub mod render;

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use bevy_replication::Replicated;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::SfxId;
use crate::core::content::media::GltfInfos;
use crate::core::content::{
    Content, ContentFile, ContentId, ContentProblem, Loading, ModuleRule, ModuleSet, NamedContent,
    Nearby, RowCheck, SchemaMark,
};
use crate::core::geometry::Capsule;
use crate::core::math::{Pos, Rng};
use crate::core::script::Value;
use crate::core::time::{UnixMillis, WallClock};
use crate::core::units::{MeterPos, Meters, Percent, Seconds};
use crate::systems::actor::{self, Hitbox, Hurt};
use crate::systems::area::{AreaId, AreaModule, AreaTag, PropId};
use crate::systems::equipment::{self, EquipmentPlace};
use crate::systems::item::{
    DropRow, Inventory, ItemDef, ItemId, ItemStack, ToolTypeDef, ToolTypeId,
};
use crate::systems::movement::{self, Bodies, Position, Spot, position};
use crate::systems::notification;
use crate::systems::player::player_requests;
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::script::effect::Audience;
use crate::systems::script::{
    EventKind, Owner, RiftEvent, RiftInstances, RunnerKind, ScriptModule, ScriptedRow,
};
use crate::systems::stat::{self, StatKind};
use crate::systems::{GameSettings, area, visibility};

pub type ResourceId = ContentId<ResourceDef>;

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_once::<Resource>()
        .replicate::<Left>()
        .replicate::<Working>()
        .add_mapped_client_message::<GatherRequest>()
        .add_client_message::<StopGathering>();
}

/// Something standing in the world that is worked until nothing is left of it: a prop to look at,
/// how much it holds, how many may work it at once, what working it takes and what it gives. Its
/// worker's gathering speed is how many times faster than stated an extraction goes, and its
/// gathering efficiency the chance that an extraction rolls its drops again: every whole hundred
/// percent is one more roll every time, and what is left over the chance of another.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ResourceDef {
    pub name: String,
    pub look: PropId,
    /// How many extractions it holds.
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub quantity: u32,
    /// How many may work it at once.
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub max_occupants: u32,
    pub extraction: Extraction,
    /// Rolled once per extraction, every row. Work needs room in the bag for the most every row
    /// can drop.
    pub drops: Vec<DropRow>,
    pub sfx: ResourceSfx,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ResourceModule>,
}

/// What one extraction takes: time alone, or a tool of a kind worn in the tool slot as well.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Extraction {
    ByHand { takes: Seconds },
    Tool { tool: ToolTypeId, takes: Seconds },
}

impl Extraction {
    /// How long one extraction takes at a gathering speed of one.
    pub fn takes(self) -> Seconds {
        match self {
            Extraction::ByHand { takes } | Extraction::Tool { takes, .. } => takes,
        }
    }

    /// The kind of tool it takes, none when it is done by hand.
    pub fn tool(self) -> Option<ToolTypeId> {
        match self {
            Extraction::ByHand { .. } => None,
            Extraction::Tool { tool, .. } => Some(tool),
        }
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSfx {
    /// Heard as each extraction completes.
    pub extract: SfxId,
    /// Heard as the last extraction is taken.
    pub ran_out: SfxId,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceModule {
    Script(ScriptModule),
}

impl ContentFile for ResourceDef {
    const TABLE: &'static str = "resource";

    fn fresh(near: &Nearby) -> Result<ResourceDef, String> {
        let sound = near.file()?;
        Ok(ResourceDef {
            name: near.name(),
            look: near.file()?,
            quantity: 1,
            max_occupants: 1,
            extraction: Extraction::ByHand {
                takes: Seconds(1.0),
            },
            drops: vec![DropRow::of(near.file()?)],
            sfx: ResourceSfx {
                extract: sound,
                ran_out: sound,
            },
            modules: Vec::new(),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.at_least("/quantity", self.quantity, 1, "extractions");
        check.at_least("/max_occupants", self.max_occupants, 1, "workers");
        check.at_least("/drops", self.drops.len(), 1, "drops");
        for (index, drop) in self.drops.iter().enumerate() {
            check.at_least(format!("/drops/{index}/amount"), drop.amount, 1, "items");
        }
        check.at_least(
            "/extraction/takes",
            self.extraction.takes().0,
            0.0,
            "seconds",
        );
    }
}

impl NamedContent for ResourceDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for ResourceDef {
    type Module = ResourceModule;

    fn modules(&self) -> &[ResourceModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<ResourceModule> {
        &mut self.modules
    }

    fn rule(_: &ResourceModule) -> ModuleRule {
        ModuleRule::AnyNumber
    }
}

impl ScriptedRow for ResourceDef {
    const OWNER: RunnerKind = RunnerKind::Resource;

    fn script(module: &ResourceModule) -> Option<&ScriptModule> {
        match module {
            ResourceModule::Script(script) => Some(script),
        }
    }
}

impl ResourceDef {
    /// How long one extraction takes a worker of gathering speed `speed`.
    pub fn takes_at(&self, speed: f32) -> Seconds {
        Seconds(self.extraction.takes().0 / speed.max(f32::EPSILON))
    }

    /// The most one extraction can drop a worker of gathering efficiency `efficiency`, row by row.
    pub fn most_at(&self, efficiency: Percent) -> Vec<ItemStack> {
        let rolls = 1 + (efficiency.0.max(0.0) / 100.0).ceil() as u32;
        self.drops
            .iter()
            .map(|drop| ItemStack::new(drop.item, drop.amount.saturating_mul(rolls)))
            .collect()
    }

    /// What one extraction drops a worker of gathering efficiency `efficiency` this time, an item
    /// to a stack.
    pub fn rolled_at(&self, efficiency: Percent, rng: &mut Rng) -> Vec<ItemStack> {
        let whole = efficiency.0.max(0.0) / 100.0;
        let again = Percent(whole.fract() * 100.0).rolled(rng);
        let rolls = 1 + whole.floor() as u32 + u32::from(again);
        let mut dropped: Vec<ItemStack> = Vec::new();
        for _ in 0..rolls {
            for stack in self.drops.iter().filter_map(|drop| drop.rolled(rng)) {
                match dropped.iter_mut().find(|held| held.item == stack.item) {
                    Some(held) => held.count += stack.count,
                    None => dropped.push(stack),
                }
            }
        }
        dropped
    }
}

/// Every resource is spawned somewhere and, where it takes a tool, there is a tool of the kind.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let spawned: Vec<ResourceId> = content
        .ids::<area::AreaDef>()
        .flat_map(|area| area::modules(&content, area))
        .filter_map(|module| match module {
            AreaModule::Spawner(spawner) => Some(spawner),
            _ => None,
        })
        .flat_map(|spawner| spawner.resources())
        .collect();
    let made: Vec<ToolTypeId> = content
        .table::<ItemDef>()
        .rows()
        .filter_map(|item| item.wear()?.tool_type())
        .collect();
    let mut needed: Vec<ToolTypeId> = Vec::new();
    for (id, def) in loading.rows::<ResourceDef>(&content) {
        if !spawned.contains(&id) {
            loading.row_warning(
                id,
                "",
                ContentProblem::new(
                    "never spawned",
                    id.path(&content),
                    "a spawner that spawns it",
                ),
            );
        }
        let Some(tool) = def.extraction.tool() else {
            continue;
        };
        needed.push(tool);
        if !made.contains(&tool) {
            loading.row_warning(
                id,
                "/extraction/tool",
                ContentProblem::new(
                    "no such tool",
                    tool.path(&content),
                    "a kind of tool some item is",
                ),
            );
        }
    }
    for (id, _) in loading.rows::<ToolTypeDef>(&content) {
        if !needed.contains(&id) {
            loading.row_warning(
                id,
                "",
                ContentProblem::new(
                    "never needed",
                    id.path(&content),
                    "a resource worked with it",
                ),
            );
        }
    }
}

/// A resource standing in the world.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Resource {
    pub def: ResourceId,
}

/// How many extractions a standing resource still holds.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Left(pub u32);

/// On a character working a resource: which, with which tool from its tool slot if any, and when
/// the extraction under way began and completes by the server's clock. It holds one of the
/// resource's places.
#[derive(Component, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct Working {
    #[entities]
    pub resource: Entity,
    pub tool: Option<ItemId>,
    pub since: UnixMillis,
    pub until: UnixMillis,
}

/// How many blows a worker had taken as work began; one more ends the work.
#[derive(Component, Clone, Copy)]
struct Unhurt(u32);

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct GatherRequest {
    #[entities]
    pub target: Entity,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct StopGathering;

/// Stands a resource of `def` at `at`, holding all it holds.
pub fn spawn(world: &mut World, def: ResourceId, at: Pos<Meters>, area: AreaId) -> Entity {
    let content = world.resource::<Content>().clone();
    let row = def.get(&content);
    world
        .spawn((
            Replicated,
            Position::at(at),
            AreaTag { area },
            Resource { def },
            Left(row.quantity),
            Spot { area, at },
            Hitbox(body(&content, def)),
            actor::Name {
                name: row.name.clone(),
            },
        ))
        .id()
}

/// The room a resource takes standing: as tall as the model of its look, and as wide as the ground
/// that look takes up, so a tree is its trunk rather than its crown. A look taking no ground is as
/// wide as its model.
pub fn body(content: &Content, def: ResourceId) -> Capsule {
    let look = def.get(content).look.get(content);
    let bounds = content
        .built::<GltfInfos>()
        .0
        .get(&look.model.path())
        .map(|model| model.bounds);
    let model_width = bounds.map_or(0.0, |bounds| {
        [bounds.min.x, bounds.max.x, bounds.min.z, bounds.max.z]
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max)
    });
    Capsule {
        radius: look.footprint.reach().unwrap_or(Meters(model_width)),
        height: Meters(bounds.map_or(0.0, |bounds| bounds.max.y)),
    }
}

/// How many characters are working `resource`.
pub fn workers(world: &mut World, resource: Entity) -> u32 {
    world
        .query::<&Working>()
        .iter(world)
        .filter(|working| working.resource == resource)
        .count() as u32
}

/// The tool `worker` would work `resource` with, none for one worked by hand; or what stands in
/// the way, told as the game words it.
pub fn workable(
    world: &mut World,
    worker: Entity,
    resource: Entity,
) -> Result<Option<ItemId>, String> {
    let content = world.resource::<Content>().clone();
    let words = &content.built::<GameSettings>().words;
    let standing = world
        .get::<Resource>(resource)
        .zip(world.get::<Left>(resource))
        .filter(|(_, left)| left.0 > 0)
        .map(|(resource, _)| resource.def);
    let Some(def) = standing else {
        return Err(words.ran_out.clone());
    };
    let row = def.get(&content);
    let tool = match row.extraction.tool() {
        None => None,
        Some(kind) => {
            let worn = equipment::in_slot(world, worker, &EquipmentPlace::Tool).filter(|worn| {
                worn.get(&content).wear().and_then(|wear| wear.tool_type()) == Some(kind)
            });
            let Some(worn) = worn else {
                return Err(words.needs_tool.replace("{tool}", &kind.get(&content).name));
            };
            Some(worn)
        }
    };
    let working_it = world
        .get::<Working>(worker)
        .is_some_and(|working| working.resource == resource);
    if !working_it && workers(world, resource) >= row.max_occupants {
        return Err(words.no_room.clone());
    }
    let efficiency = Percent(stat::effective(
        world,
        worker,
        StatKind::GatheringEfficiency,
    ));
    let fits = world
        .get::<Inventory>(worker)
        .cloned()
        .unwrap_or_else(Inventory::empty)
        .exchange(&content, &[], &row.most_at(efficiency));
    match fits {
        Ok(()) => Ok(tool),
        Err(_) => Err(words.bag_room.clone()),
    }
}

pub fn gather_request(world: &mut World) {
    player_requests::<GatherRequest>(world, true, |world, player, request| {
        let target = request.target;
        if world.get::<Resource>(target).is_none() || !visibility::in_sight(world, player, target) {
            return Ok(());
        }
        workable(world, player, target)?;
        stop(world, player);
        reach::intend(world, player, ReachTarget::Actor(target), ReachAct::Gather);
        Ok(())
    });
    player_requests::<StopGathering>(world, false, |world, player, _| {
        stop(world, player);
        Ok(())
    });
}

/// Walks every character that set out for a resource into reach of it, and starts the work there.
pub fn gathering(world: &mut World, intents: &mut reach::Intents) {
    for (worker, target) in reach::intending(world, intents, ReachAct::Gather) {
        let Some(reach) = work_reach(world, target) else {
            reach::forget(world, worker);
            continue;
        };
        match reach::pursue(world, worker, reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, worker),
            Pursuit::Arrived => {
                reach::forget(world, worker);
                start(world, worker, target);
            }
        }
    }
}

type Workers = QueryState<(Entity, &'static Working)>;

/// Ends the work of whoever no longer works, then completes the extractions that are due, in the
/// order they came due and then in the order their workers began, so what is left is spent once.
pub fn work(world: &mut World, workers: &mut Workers) {
    let now = world.resource::<WallClock>().now;
    let content = world.resource::<Content>().clone();
    let working: Vec<(Entity, Working)> = workers
        .iter(world)
        .map(|(worker, working)| (worker, *working))
        .collect();
    let mut due: Vec<(UnixMillis, UnixMillis, Entity)> = Vec::new();
    for (worker, working) in working {
        if let Some(why) = interrupted(world, worker, &working) {
            stop(world, worker);
            if let Some(why) = why {
                notification::error(world, worker, why);
            }
        } else if working.until <= now {
            due.push((working.until, working.since, worker));
        }
    }
    due.sort_unstable();
    for (done, _, worker) in due {
        extract(world, &content, worker, done);
    }
}

/// Ends the work `worker` has under way, if any; the extraction under way takes and gives nothing.
pub fn stop(world: &mut World, worker: Entity) {
    if let Ok(mut worker) = world.get_entity_mut(worker) {
        worker.remove::<(Working, Unhurt)>();
    }
}

/// How near a resource its workers stand: against the body of its look.
fn work_reach(world: &World, resource: Entity) -> Option<Meters> {
    let body = world.get::<Hitbox>(resource)?.0;
    Some(Meters(
        body.radius.0 + world.resource::<Bodies>().spacing().0,
    ))
}

fn start(world: &mut World, worker: Entity, resource: Entity) {
    let tool = match workable(world, worker, resource) {
        Ok(tool) => tool,
        Err(why) => return notification::error(world, worker, why),
    };
    if let Some(at) = position(world, resource) {
        actor::face_toward(world, worker, at);
    }
    let since = world.resource::<WallClock>().now;
    let blows = world.get::<Hurt>(worker).map_or(0, |hurt| hurt.0);
    world.entity_mut(worker).insert(Unhurt(blows));
    begin(world, worker, resource, tool, since);
}

/// Begins an extraction of `resource` at `since`, due as soon as the worker's gathering speed makes
/// it.
fn begin(
    world: &mut World,
    worker: Entity,
    resource: Entity,
    tool: Option<ItemId>,
    since: UnixMillis,
) {
    let Some(def) = world.get::<Resource>(resource).map(|resource| resource.def) else {
        return;
    };
    let speed = stat::effective(world, worker, StatKind::GatheringSpeed);
    let takes = def.get(world.resource::<Content>()).takes_at(speed);
    world.entity_mut(worker).insert(Working {
        resource,
        tool,
        since,
        until: since.after(takes),
    });
}

/// Why the work of `worker` is over without its doing, if it is: `Some(None)` where the worker
/// knows already, `Some(Some(_))` with what to tell them.
fn interrupted(world: &mut World, worker: Entity, working: &Working) -> Option<Option<String>> {
    let content = world.resource::<Content>().clone();
    let standing = world
        .get::<Left>(working.resource)
        .is_some_and(|left| left.0 > 0);
    if !standing {
        return Some(Some(content.built::<GameSettings>().words.ran_out.clone()));
    }
    let blows = world.get::<Hurt>(worker).map_or(0, |hurt| hurt.0);
    let unhurt = world
        .get::<Unhurt>(worker)
        .is_some_and(|then| then.0 == blows);
    let in_reach = work_reach(world, working.resource)
        .zip(position(world, worker))
        .zip(position(world, working.resource))
        .is_some_and(|((reach, at), resource)| at.distance(resource) <= reach);
    let tool_worn = working.tool.is_none()
        || equipment::in_slot(world, worker, &EquipmentPlace::Tool) == working.tool;
    let busy = movement::moving(world, worker) || reach::current(world, worker).is_some();
    (stat::is_dead(world, worker) || !unhurt || !in_reach || !tool_worn || busy).then_some(None)
}

/// Completes the extraction `worker` has under way, which came due at `done`: takes one from the
/// resource, rolls its drops into the worker's bag as many times as its efficiency makes it, and begins the next where the bag has room.
fn extract(world: &mut World, content: &Content, worker: Entity, done: UnixMillis) {
    let Some(working) = world.get::<Working>(worker).copied() else {
        return;
    };
    let resource = working.resource;
    let standing = world
        .get::<Resource>(resource)
        .zip(world.get::<Left>(resource))
        .filter(|(_, left)| left.0 > 0)
        .map(|(resource, left)| (resource.def, left.0));
    let Some((def, left)) = standing else {
        stop(world, worker);
        let ran_out = content.built::<GameSettings>().words.ran_out.clone();
        return notification::error(world, worker, ran_out);
    };
    let row = def.get(content);
    let efficiency = Percent(stat::effective(
        world,
        worker,
        StatKind::GatheringEfficiency,
    ));
    let dropped = world.resource_scope(|_, mut rng: Mut<Rng>| row.rolled_at(efficiency, &mut rng));
    let paid = world
        .get_mut::<Inventory>(worker)
        .is_some_and(|mut bag| bag.exchange(content, &[], &dropped).is_ok());
    if !paid {
        return stop(world, worker);
    }
    world.entity_mut(resource).insert(Left(left - 1));
    notification::play_sound(
        world,
        Some(resource),
        Some(worker),
        row.sfx.extract,
        Audience::Everyone,
    );
    for stack in &dropped {
        raise(world, def, EventKind::Extracted, worker, Some(*stack));
    }
    if left == 1 {
        notification::play_sound(
            world,
            Some(resource),
            Some(worker),
            row.sfx.ran_out,
            Audience::Everyone,
        );
        raise(world, def, EventKind::RanOut, worker, None);
        stop(world, worker);
        world.despawn(resource);
        return;
    }
    match workable(world, worker, resource) {
        Ok(tool) => begin(world, worker, resource, tool, done),
        Err(why) => {
            stop(world, worker);
            notification::error(world, worker, why);
        }
    }
}

fn raise(
    world: &mut World,
    def: ResourceId,
    kind: EventKind,
    worker: Entity,
    stack: Option<ItemStack>,
) {
    let fields = stack.map_or_else(Vec::new, |stack| {
        vec![
            Value::Int(i64::from(stack.item.raw())),
            Value::Int(i64::from(stack.count)),
        ]
    });
    world.resource_mut::<RiftInstances>().raise(
        Owner::Row(RunnerKind::Resource, def.raw()),
        kind,
        Some(worker),
        fields,
        move |event| match event {
            RiftEvent::Extracted { item } => {
                item.is_none_or(|item| stack.is_some_and(|stack| stack.item == item))
            }
            RiftEvent::RanOut => true,
            _ => false,
        },
    );
}

/// Brings a worker's tool out into its hands as it begins to work.
pub fn take_out(insert: On<Insert<Working>>, world: &World, mut commands: Commands) {
    equipment::show(world, insert.entity, true, &mut commands);
}

/// Puts a worker's tool away as its work ends.
pub fn put_away(remove: On<Remove<Working>>, world: &World, mut commands: Commands) {
    equipment::show(world, remove.entity, false, &mut commands);
}
