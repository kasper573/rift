//! A server world run inside the editor, for the previews that need the game's own mechanics: a
//! status a designer applies to an actor, a skill cast on a loop with the scene it needs set up
//! around the caster. It steps at the server's tick rate in real time; the stage mirrors what it
//! holds.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use bevy::prelude::*;
use engine::core::content::Content;
use engine::core::geometry::{PlacedShape, Shape};
use engine::core::math::Pos;
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::core::units::{MeterPos, Meters};
use engine::core::vfx::VfxInstance;
use engine::systems::actor::plate::{self, PlateLabel};
use engine::systems::actor::{Action, Actor, ModelId};
use engine::systems::area::{self, AreaId, AreaModule};
use engine::systems::class::ClassDef;
use engine::systems::item::{ItemDef, ItemId, ItemKind, ItemStack};
use engine::systems::movement::Position;
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, SnapshotSkill};
use engine::systems::player::{ClientId, JoinRequest, Players, SpawnPolicy, SpawnRules};
use engine::systems::script::{Condition, Place, RiftQuery};
use engine::systems::skill::casting::{self, CastAim};
use engine::systems::skill::{self, Rank, SkillDef, SkillId, Targeting};
use engine::systems::stat::{self, StatKind, Stats};
use engine::systems::status::{self, StatusEffectId};
use engine::systems::{GameSettings, TICK_HZ, from_client, server_app, start_server};

use super::{Scenario, spread};

const SEED: u64 = 0x57a6e;
const TICKS_PER_FRAME_AT_MOST: u32 = 6;
const REST_BETWEEN_CASTS: Seconds = Seconds(1.0);
const WAIT_FOR_A_VFX: Seconds = Seconds(3.0);
const TARGET_REACH: f32 = 2.0;
const AREA_REACH: f32 = 3.0;
const CROWD: u32 = 3;
const CLEAR_OF_CASTER: f32 = 1.3;
const STURDY: f32 = 1_000_000.0;
const STATUS_POWER: f32 = 1.0;

pub struct LiveWorld {
    world: World,
    clock: WallClock,
    carry: Duration,
    pub generation: u64,
    setup: Setup,
    /// What the scenario is up to, in words for the window: the cast count, or why a cast refuses.
    pub report: String,
    /// Raised once the scene no longer holds (a target fell), so it is built again.
    pub stale: bool,
}

enum Setup {
    Status {
        actor: Entity,
        effect: StatusEffectId,
    },
    Skill {
        caster: Entity,
        skill: SkillId,
        targets: Vec<Entity>,
        aim: CastAim,
        phase: Phase,
        casts: u32,
    },
}

enum Phase {
    Resting { until: UnixMillis },
    Playing { since: UnixMillis, seen: bool },
}

/// An actor of the live world as the stage draws it.
pub struct LiveActor {
    pub entity: Entity,
    pub actor: Actor,
    pub at: Pos<Meters>,
    pub plate: Option<PlateLabel>,
}

impl LiveWorld {
    pub fn build(
        content: &Content,
        scenario: &Scenario,
        generation: u64,
    ) -> Result<LiveWorld, String> {
        let settings = content.built::<GameSettings>();
        let spawn = settings.initial_spawn;
        let clock = WallClock {
            now: SystemTime::now().into(),
            reset: UtcHour::MIDNIGHT,
        };
        let mut world = sandbox(content, spawn.area, SEED, clock);
        let setup = match scenario {
            Scenario::Status { model, effect, .. } => Setup::Status {
                actor: spawn_sturdy(&mut world, content, *model, spawn.at, spawn.area),
                effect: *effect,
            },
            Scenario::Skill { skill } => skill_setup(&mut world, content, *skill, spawn, clock)?,
        };
        let mut live = LiveWorld {
            world,
            clock,
            carry: Duration::ZERO,
            generation,
            setup,
            report: String::new(),
            stale: false,
        };
        live.reconcile(scenario);
        Ok(live)
    }

    /// Runs the world for `real` more time, a tick at a time, and the scenario with it.
    pub fn step(&mut self, real: Duration) {
        let period = TICK_HZ.period();
        self.carry = (self.carry + real).min(period * TICKS_PER_FRAME_AT_MOST);
        while self.carry >= period {
            self.carry -= period;
            self.clock.now = self.clock.now.after(Seconds(period.as_secs_f32()));
            self.world.insert_resource(self.clock);
            tick(&mut self.world);
            self.scenario_tick();
        }
    }

    /// Brings the world in line with what the scene asks: the status on or off the actor.
    pub fn reconcile(&mut self, scenario: &Scenario) {
        if let (Setup::Status { actor, effect }, Scenario::Status { applied, .. }) =
            (&self.setup, scenario)
        {
            let (actor, effect) = (*actor, *effect);
            let has = self
                .world
                .get::<status::Statuses>(actor)
                .is_some_and(|statuses| statuses.has(effect));
            if *applied && !has {
                status::apply(&mut self.world, actor, effect, STATUS_POWER, None);
            } else if !*applied && has {
                status::remove(&mut self.world, actor, effect);
            }
        }
    }

    pub fn now(&self) -> UnixMillis {
        self.clock.now
    }

    pub fn actors(&self) -> Vec<LiveActor> {
        let content = self.world.resource::<Content>();
        self.cast()
            .into_iter()
            .filter_map(|entity| {
                Some(LiveActor {
                    entity,
                    actor: self.world.get::<Actor>(entity)?.clone(),
                    at: self.world.get::<Position>(entity)?.pos,
                    plate: plate::label_of(&self.world, content, entity),
                })
            })
            .collect()
    }

    pub fn vfx(&mut self) -> Vec<(Entity, VfxInstance)> {
        self.world
            .query::<(Entity, &VfxInstance)>()
            .iter(&self.world)
            .map(|(entity, instance)| (entity, instance.clone()))
            .collect()
    }

    fn cast(&self) -> Vec<Entity> {
        match &self.setup {
            Setup::Status { actor, .. } => vec![*actor],
            Setup::Skill {
                caster, targets, ..
            } => std::iter::once(*caster)
                .chain(targets.iter().copied())
                .collect(),
        }
    }

    fn scenario_tick(&mut self) {
        let now = self.clock.now;
        let Setup::Skill {
            caster,
            skill,
            targets,
            aim,
            phase,
            casts,
        } = &mut self.setup
        else {
            return;
        };
        if targets
            .iter()
            .any(|target| stat::is_dead(&self.world, *target))
        {
            self.stale = true;
            return;
        }
        let content = self.world.resource::<Content>().clone();
        let def = skill.get(&content);
        match phase {
            Phase::Resting { until } if now >= *until => {
                if let Some(mut inventory) = self
                    .world
                    .get_mut::<engine::systems::item::Inventory>(*caster)
                {
                    let _ = inventory.exchange(&content, &[], &def.cost);
                }
                if let Some(mut cooldowns) = self.world.get_mut::<skill::Cooldowns>(*caster) {
                    cooldowns.0.remove(skill);
                }
                match casting::cast(&mut self.world, *caster, *skill, *aim, false) {
                    Ok(()) => {
                        *casts += 1;
                        *phase = Phase::Playing {
                            since: now,
                            seen: false,
                        };
                        self.report = format!("{} · cast {casts}", def.name);
                    }
                    Err(reason) => self.report = reason,
                }
            }
            Phase::Resting { .. } => {}
            Phase::Playing { since, seen } => {
                let since = *since;
                let mut playing = false;
                let mut query = self.world.query::<&VfxInstance>();
                for instance in query.iter(&self.world) {
                    if instance.started >= since {
                        *seen = true;
                        playing |= instance.until > now;
                    }
                }
                let busy = self
                    .world
                    .get::<Actor>(*caster)
                    .is_some_and(|actor| actor.action != Action::Idle);
                let waited_enough = *seen || now.since(since) > WAIT_FOR_A_VFX;
                if !playing && !busy && waited_enough {
                    *phase = Phase::Resting {
                        until: now.after(REST_BETWEEN_CASTS),
                    };
                }
            }
        }
    }
}

/// A server world over `content` opened on `area`, where nobody dies and nobody has joined yet.
pub fn sandbox(content: &Content, area: AreaId, seed: u64, clock: WallClock) -> World {
    let mut app = server_app(area, 0, seed, clock, content);
    app.insert_resource(SpawnRules {
        policy: SpawnPolicy::Map,
        immortal: true,
    });
    start_server(&mut app);
    std::mem::take(app.world_mut())
}

pub struct Joined {
    pub conn: Entity,
    pub actor: Entity,
}

/// Joins a character as a player would, placed at `start`.
pub fn join(
    world: &mut World,
    snapshot: CharacterSnapshot,
    start: Place,
    client: ClientId,
) -> Result<Joined, String> {
    let conn = world.spawn((client, JoinAs { snapshot, start })).id();
    from_client(world, conn, JoinRequest);
    tick(world);
    let actor = world
        .resource::<Players>()
        .0
        .get(&client)
        .copied()
        .ok_or("the character did not join")?;
    Ok(Joined { conn, actor })
}

pub fn tick(world: &mut World) {
    world.run_schedule(bevy::app::Main);
    world.clear_trackers();
}

/// The scene a skill needs to be cast: a character who knows it, wearing and carrying what it
/// asks, with whatever it targets standing before them.
fn skill_setup(
    world: &mut World,
    content: &Content,
    skill: SkillId,
    spawn: Place,
    clock: WallClock,
) -> Result<Setup, String> {
    let def = skill.get(content);
    let settings = content.built::<GameSettings>();
    let mut snapshot =
        CharacterSnapshot::of_player(content, "Caster", settings.player.get(content));
    match taught_by(content, skill) {
        Some((class, node)) => {
            let levels = &class.get(content).levels;
            snapshot.class = class;
            snapshot.xp = levels.iter().map(|level| level.xp).max().unwrap_or(0);
            snapshot.skills = vec![SnapshotSkill {
                node,
                rank: Rank::FIRST,
            }];
        }
        None => match granted_by(content, skill) {
            Some(item) => carry(&mut snapshot, content, item),
            None => {
                return Err(format!(
                    "nothing teaches {}: no class tree has it and no gear grants it",
                    def.name
                ));
            }
        },
    }
    for item in required_items(def) {
        carry(&mut snapshot, content, item);
    }
    for stack in &def.cost {
        snapshot.bag.push(ItemStack::new(stack.item, stack.count));
    }
    let area = area::load(content, spawn.area);
    let free = |at: Pos<Meters>| area.mesh.nearest(at).unwrap_or(at);
    let caster = join(world, snapshot.clone(), spawn, ClientId(1))?.actor;
    let ally = || {
        let mut friend =
            CharacterSnapshot::of_player(content, "Friend", settings.player.get(content));
        friend.start = snapshot.start;
        friend
    };
    let beside = Pos::new(spawn.at.x + TARGET_REACH, spawn.at.y);
    let (targets, aim) = match def.target {
        Targeting::Caster => (Vec::new(), CastAim::None),
        Targeting::SingleEnemy => {
            let enemy = spawn_sturdy(
                world,
                content,
                enemy_model(content, spawn.area),
                free(beside),
                spawn.area,
            );
            (vec![enemy], CastAim::Actor(enemy))
        }
        Targeting::SingleAlly => {
            let friend = join(
                world,
                ally(),
                Place {
                    at: free(beside),
                    ..spawn
                },
                ClientId(2),
            )?
            .actor;
            (vec![friend], CastAim::Actor(friend))
        }
        Targeting::AreaEnemy | Targeting::AreaAlly => {
            let point = Pos::new(spawn.at.x + AREA_REACH, spawn.at.y);
            let shape = def.area.unwrap_or(Shape::Circle {
                radius: Meters(1.5),
            });
            let footprint = PlacedShape::cast(shape, spawn.at, point);
            let mut targets = Vec::new();
            let spots: Vec<Pos<Meters>> = spread(&footprint, CROWD * 2)
                .into_iter()
                .filter(|spot| spot.distance(spawn.at).0 >= CLEAR_OF_CASTER)
                .take(CROWD as usize)
                .collect();
            for (index, spot) in spots.into_iter().enumerate() {
                let at = free(spot);
                targets.push(if def.target == Targeting::AreaEnemy {
                    spawn_sturdy(
                        world,
                        content,
                        enemy_model(content, spawn.area),
                        at,
                        spawn.area,
                    )
                } else {
                    join(
                        world,
                        ally(),
                        Place { at, ..spawn },
                        ClientId(2 + index as u32),
                    )?
                    .actor
                });
            }
            (targets, CastAim::Point(point))
        }
    };
    Ok(Setup::Skill {
        caster,
        skill,
        targets,
        aim,
        phase: Phase::Resting {
            until: clock.now.after(REST_BETWEEN_CASTS),
        },
        casts: 0,
    })
}

/// An actor of a model that no cast brings down, so the scene stands through every repeat.
fn spawn_sturdy(
    world: &mut World,
    content: &Content,
    model: ModelId,
    at: Pos<Meters>,
    area: AreaId,
) -> Entity {
    let def = NpcDef {
        name: model.name(content).to_owned(),
        model,
        bust: None,
        stats: Stats(BTreeMap::from([
            (StatKind::MaxHealth, STURDY),
            (StatKind::Health, STURDY),
        ])),
        basic_attack: None,
        skills: Vec::new(),
        wearing: Vec::new(),
        modules: Vec::new(),
    };
    npc::spawn_actor(world, &def, at, area)
}

/// The class whose tree teaches the skill, and the node that does.
fn taught_by(
    content: &Content,
    skill: SkillId,
) -> Option<(
    engine::systems::class::ClassId,
    engine::systems::class::NodeKey,
)> {
    content.table::<ClassDef>().ids().find_map(|class| {
        class
            .get(content)
            .tree
            .0
            .iter()
            .find(|(_, node)| node.skill == skill)
            .map(|(key, _)| (class, key.clone()))
    })
}

fn granted_by(content: &Content, skill: SkillId) -> Option<ItemId> {
    content
        .table::<ItemDef>()
        .ids()
        .find(|item| skill::grants_of(content, *item).contains(&skill))
}

/// The items a skill's requirements ask the caster to wear or hold.
fn required_items(def: &SkillDef) -> Vec<ItemId> {
    let mut items = Vec::new();
    if let Some(requires) = &def.requires {
        requires.visit(&mut |expr| {
            if let Condition::Query(
                RiftQuery::Equipped { item, .. } | RiftQuery::Holding { item, .. },
            ) = expr
            {
                items.push(*item);
            }
        });
    }
    items
}

/// Gives the character an item the way it is used: worn when it is gear, in the bag otherwise.
fn carry(snapshot: &mut CharacterSnapshot, content: &Content, item: ItemId) {
    match item.get(content).kind {
        ItemKind::Equipment { .. } => snapshot.equipped.push(item),
        ItemKind::Usable | ItemKind::Resource => snapshot.bag.push(ItemStack::new(item, 1)),
    }
}

/// What an enemy looks like here: whatever the spawn area's populations are made of, else the
/// first npc there is.
fn enemy_model(content: &Content, zone: AreaId) -> ModelId {
    area::modules(content, zone)
        .find_map(|module| match module {
            AreaModule::Population(population) => Some(population.npc.get(content).model),
            _ => None,
        })
        .or_else(|| {
            content
                .table::<NpcDef>()
                .ids()
                .next()
                .map(|npc| npc.get(content).model)
        })
        .unwrap_or_else(|| content.built::<GameSettings>().player.get(content).model)
}
