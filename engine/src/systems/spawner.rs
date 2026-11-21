use std::collections::BTreeSet;

use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, RowCheck, SchemaMark, serial};
use crate::core::content::{ContentProblem, Loading};
use crate::core::geometry::corners;
use crate::core::ground::TileAt;
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::script::{
    Arrow, Compiler, EffectCall, EffectDef, EffectStep, EventDef, EventKind, Instances, NEXT,
    ScriptEvent, ScriptFault, Scripts, Value, ValueType, Vocabulary,
};
use crate::core::script::{RunnerKind, owner_site};
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::area::AREA_RUNNER;
use crate::systems::area::sky;
use crate::systems::area::sky::SkyCondition;
use crate::systems::area::{self, Area, AreaId, AreaModule, TimeOfDayName, Wild};
use crate::systems::movement::{NavMesh, Position, Spots, position};
use crate::systems::npc::{self, AbsenceSpanAt, Npc, NpcId, Pack, Roams, absence};
use crate::systems::player::PlayedBy;
use crate::systems::resource::{self, ResourceAfloat, ResourceId};
use crate::systems::visibility::Viewers;
use crate::systems::{WorldArea, dialogue, stat, visibility};

/// Keeps what it spawns standing within a rect of the area, from one of its populations at a
/// time: an endless one for as long as it is there, a finite one until all it spawned has fallen.
#[derive(JsonSchema, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spawner {
    /// The name the area's scripts hear it by, in its events and to wake it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What it spawns, one population at a time, each numbered from 1 in this order.
    pub populations: Vec<Population>,
    /// In which order its populations take their turns.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub schedule: Schedule,
    /// Whether a population that is exhausted may take a turn again. Without it, the spawner is
    /// exhausted once none is left.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub repopulate: bool,
    /// How long after one population is exhausted the next one begins.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub delay_between: Seconds,
    /// Whether it spawns nothing until a script wakes it by its name.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub dormant: bool,
    /// The whole area without one.
    #[schemars(with = "Option<serial::RectRow>")]
    #[serde(
        default,
        serialize_with = "serial::opt_rect",
        deserialize_with = "serial::opt_rect_from",
        skip_serializing_if = "Option::is_none"
    )]
    pub within: Option<Rect<Meters>>,
    /// What its entries are placed on. Only resources lie on water.
    #[serde(default, skip_serializing_if = "serial::is_default")]
    pub on: SpawnSurface,
}

/// Wakes the dormant spawners of this world named `spawner`: the first population of each takes
/// its turn at once.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("spawner"))]
#[serde(deny_unknown_fields)]
pub struct WakeSpawner {
    pub spawner: String,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for WakeSpawner {
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, _cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        if self.spawner.trim().is_empty() {
            return Err(ScriptFault::new(
                "no spawner",
                "the name of a spawner to wake",
            ));
        }
        let name = self.spawner.clone();
        Ok(Box::new(move |world, _, _| {
            wake(world, &name);
            EffectStep::Done
        }))
    }
}

/// One population of a spawner: what it spawns, how many stand at once, and how many in all.
#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Population {
    /// What an entry may be. Every fill draws one, by weight, of those whose `when` holds.
    pub spawns: Vec<SpawnShare>,
    /// How many it spawns in all, after which it is exhausted once every one of them has fallen.
    /// Endless without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub size: Option<u32>,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub max_spawned_at_once: u32,
    /// How long a freed entry stays empty. Without one it is never filled again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respawn: Option<Seconds>,
}

/// In which order a spawner's populations take their turns.
#[derive(JsonSchema, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    /// In the order the spawner lists them, from the first.
    #[default]
    Ordered,
    /// One drawn at random each turn.
    Random,
}

/// What a spawner places its entries on.
#[derive(JsonSchema, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpawnSurface {
    /// The ground everyone walks.
    #[default]
    Ground,
    /// Wherever water stands, clear of every prop.
    Water,
}

#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpawnShare {
    #[serde(flatten)]
    pub what: Spawns,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub weight: u32,
    /// The sky it is drawn under, its weight counting for as much as the condition holds; always
    /// without one. What stands of it once it holds no longer leaves when nobody is looking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<SkyCondition>,
}

impl SpawnShare {
    pub fn of(what: Spawns) -> SpawnShare {
        SpawnShare {
            what,
            weight: 1,
            when: None,
        }
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Spawns {
    Npc { npc: NpcId },
    Resource { resource: ResourceId },
}

impl Spawns {
    pub fn path(self, content: &Content) -> &str {
        match self {
            Spawns::Npc { npc } => npc.path(content),
            Spawns::Resource { resource } => resource.path(content),
        }
    }
}

impl Spawner {
    /// One endless population of one thing, one at a time.
    pub fn of(what: Spawns, within: Option<Rect<Meters>>) -> Spawner {
        Spawner {
            name: None,
            populations: vec![Population::of(what)],
            schedule: Schedule::Ordered,
            repopulate: false,
            delay_between: Seconds(0.0),
            dormant: false,
            within,
            on: SpawnSurface::Ground,
        }
    }

    /// Every share of every population.
    pub fn shares(&self) -> impl Iterator<Item = &SpawnShare> + '_ {
        self.populations
            .iter()
            .flat_map(|population| &population.spawns)
    }

    pub fn npcs(&self) -> impl Iterator<Item = NpcId> + '_ {
        self.shares().filter_map(|share| match share.what {
            Spawns::Npc { npc } => Some(npc),
            Spawns::Resource { .. } => None,
        })
    }

    pub fn resources(&self) -> impl Iterator<Item = ResourceId> + '_ {
        self.shares().filter_map(|share| match share.what {
            Spawns::Resource { resource } => Some(resource),
            Spawns::Npc { .. } => None,
        })
    }

    /// The most entries any of its populations keeps standing at once.
    pub fn most_at_once(&self) -> u32 {
        self.populations
            .iter()
            .map(|population| population.max_spawned_at_once)
            .max()
            .unwrap_or(0)
    }

    /// Whether what it spawns keeps out of the safe zones, as wild npcs do.
    pub fn keeps_wild(&self) -> bool {
        self.npcs().next().is_some()
    }

    /// The files it spawns, as a problem names them.
    pub fn named(&self, content: &Content) -> String {
        let mut paths: Vec<&str> = Vec::new();
        for path in self.shares().map(|share| share.what.path(content)) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        paths.join(", ")
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.at_least(
            format!("{pointer}/populations"),
            self.populations.len(),
            1,
            "populations",
        );
        cx.at_least(
            format!("{pointer}/delay_between"),
            self.delay_between.0,
            0.0,
            "seconds",
        );
        match &self.name {
            Some(name) => cx.text(format!("{pointer}/name"), name, 1, 40),
            None if self.dormant => cx.error(
                format!("{pointer}/name"),
                "dormant without a name",
                "a dormant spawner no script can wake",
                "a name the area's scripts wake it by",
            ),
            None => {}
        }
        for (index, population) in self.populations.iter().enumerate() {
            population.check(&format!("{pointer}/populations/{index}"), self.on, cx);
        }
    }
}

impl Population {
    /// An endless population of one thing, one at a time, never filled again once it falls.
    pub fn of(what: Spawns) -> Population {
        Population {
            spawns: vec![SpawnShare::of(what)],
            size: None,
            max_spawned_at_once: 1,
            respawn: None,
        }
    }

    pub fn check(&self, pointer: &str, on: SpawnSurface, cx: &mut RowCheck) {
        cx.at_least(
            format!("{pointer}/max_spawned_at_once"),
            self.max_spawned_at_once,
            1,
            "entries",
        );
        cx.at_least(
            format!("{pointer}/spawns"),
            self.spawns.len(),
            1,
            "things to spawn",
        );
        if let Some(size) = self.size {
            cx.at_least(
                format!("{pointer}/size"),
                size,
                self.max_spawned_at_once,
                "spawned in all, as many as stand at once",
            );
            if size > self.max_spawned_at_once && self.respawn.is_none() {
                cx.error(
                    format!("{pointer}/respawn"),
                    "never spawns its size",
                    "more in all than stand at once, and no respawn to fill a freed entry",
                    "a respawn, after which a freed entry is filled again",
                );
            }
        }
        for (index, share) in self.spawns.iter().enumerate() {
            cx.at_least(
                format!("{pointer}/spawns/{index}/weight"),
                share.weight,
                1,
                "shares",
            );
            if on == SpawnSurface::Water && matches!(share.what, Spawns::Npc { .. }) {
                cx.error(
                    format!("{pointer}/spawns/{index}"),
                    "walks on water",
                    "an npc in a spawner that places on water",
                    "resources alone on water: an npc walks the ground",
                );
            }
            if self.spawns[..index]
                .iter()
                .any(|other| other.what == share.what)
            {
                cx.error(
                    format!("{pointer}/spawns/{index}"),
                    "listed twice",
                    "the same thing on two lines",
                    "each thing once, with its whole share",
                );
            }
        }
        match self.respawn {
            Some(respawn) => cx.at_least(format!("{pointer}/respawn"), respawn.0, 0.0, "seconds"),
            None if self.follows_sky() => cx.error(
                format!("{pointer}/respawn"),
                "needs a respawn",
                "a share that follows the sky, and no respawn",
                "a respawn, over which the spawner turns from one share to the next",
            ),
            None => {}
        }
    }

    /// Whether any of what it spawns comes and goes with the sky.
    pub fn follows_sky(&self) -> bool {
        self.spawns.iter().any(|share| share.when.is_some())
    }

    /// Which of its shares an entry is filled from, each as likely as its weight times how much
    /// of it `holds`; none while nothing of any holds.
    pub fn draw(&self, rng: &mut Rng, holds: impl Fn(&SpawnShare) -> f32) -> Option<ShareAt> {
        let odds: Vec<f32> = self
            .spawns
            .iter()
            .map(|share| share.weight as f32 * holds(share))
            .collect();
        let total: f32 = odds.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut drawn = rng.rand_float() * total;
        let last = odds.iter().rposition(|&odds| odds > 0.0)?;
        let at = odds[..last]
            .iter()
            .position(|&odds| {
                let within = drawn < odds;
                drawn -= odds;
                within
            })
            .unwrap_or(last);
        Some(ShareAt(at as u32))
    }
}

/// Which of a spawner's shares, by its place among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShareAt(pub u32);

impl ShareAt {
    pub fn of(self, population: &Population) -> &SpawnShare {
        &population.spawns[self.0 as usize]
    }
}

/// Which of its area's modules keeps an entry: the place of a spawner or a resident in the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpawnerAt(pub u32);

/// On whatever a spawner or a resident's place brought to life; removing it frees the entry.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable, on_remove = free_entry)]
pub struct Spawned {
    pub spawner: SpawnerAt,
    pub entry: u32,
    /// The turn of its spawner's populations it was spawned in: once another population takes
    /// its turn, freeing it frees nothing.
    pub turn: u32,
}

/// The entries of every spawner and resident the world has reached.
#[derive(Resource, Default)]
pub struct SpawnerEntries {
    kept: HashMap<SpawnerAt, Entries>,
    /// The entries that stand empty, in their order in the scene: all a tick may have to fill.
    empty: BTreeSet<(SpawnerAt, u32)>,
    /// Those of them whose entries come and go with the sky, in their order in the scene.
    under_sky: Vec<SpawnerAt>,
    /// The sky those were last tended under, as far as it matters to them: a tick under the
    /// same one has nothing to take away and nothing idle to wake.
    tended: SkyTended,
    /// The spawners whose course may still move on from the population it is at: those with a
    /// finite population, or one that waits.
    moving: BTreeSet<SpawnerAt>,
    /// The names scripts have woken, so a dormant spawner reached after its waking wakes too.
    woken: BTreeSet<String>,
}

#[derive(Default)]
struct SkyTended {
    /// Each condition a share follows, once, with whether anything of it held.
    followed: Vec<(SkyCondition, bool)>,
    /// The time of day, where an npc that keeps hours is among what is kept.
    time: Option<Option<TimeOfDayName>>,
    /// Whether something was left to see to under that sky: what goes once nobody looks, or
    /// whoever is away but for those in the middle of something with it.
    unsettled: bool,
}

struct Entries {
    pack: Pack,
    entries: Vec<Entry>,
    /// The sign a resident that is away has left where it lives, and for which span of its
    /// absence.
    sign: Option<(Entity, AbsenceSpanAt)>,
    /// Where a spawner stands among its populations; a resident has none.
    course: Option<Course>,
}

/// Where a spawner stands among its populations.
#[derive(Clone)]
struct Course {
    stage: Stage,
    /// The population that last took a turn, which an ordered schedule goes on from.
    last: Option<usize>,
    /// How many the population at its turn has spawned.
    spawned: u32,
    /// How many turns its populations have taken.
    turn: u32,
    /// Which populations are exhausted.
    exhausted: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// Waiting for a script to wake it.
    Dormant,
    /// The population at its turn.
    Active(usize),
    /// The population whose turn begins at `at`.
    Waiting { next: usize, at: Seconds },
    /// No population is left to take a turn.
    Exhausted,
}

impl Course {
    fn active(&self) -> Option<usize> {
        match self.stage {
            Stage::Active(population) => Some(population),
            _ => None,
        }
    }

    /// The population to take the next turn, if any is left to.
    fn next(&self, spawner: &Spawner, rng: &mut Rng) -> Option<usize> {
        let count = spawner.populations.len();
        let open = |population: &usize| spawner.repopulate || !self.exhausted[*population];
        match spawner.schedule {
            Schedule::Ordered => {
                let from = self.last.map_or(0, |last| last + 1);
                (0..count).map(|step| (from + step) % count).find(open)
            }
            Schedule::Random => {
                let open: Vec<usize> = (0..count).filter(open).collect();
                let drawn = (rng.rand_float() * open.len() as f32) as usize;
                open.get(drawn.min(open.len().saturating_sub(1))).copied()
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Entry {
    /// Its population spawned all it spawns: it is never filled again.
    Spent,
    /// Filled by what `stands`, drawn from `share`; a resident is of none.
    Filled {
        share: Option<ShareAt>,
        stands: Entity,
    },
    /// Empty since `since`, to be filled again after `wait`, or else its spawner's whole respawn.
    Freed {
        since: Seconds,
        wait: Option<Seconds>,
    },
    /// Empty, as nothing of its spawner holds under the sky, or its resident is away.
    Idle,
}

impl SpawnerEntries {
    /// How many of the spawner's entries stand filled.
    pub fn filled(&self, spawner: SpawnerAt) -> usize {
        self.kept.get(&spawner).map_or(0, |entries| {
            entries
                .entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Filled { .. }))
                .count()
        })
    }

    /// Keeps the entries of the module at `at`: a resident's one, empty, and none yet of a
    /// spawner, whose population fills them as it takes its turn. Looks to the sky for them when
    /// the module has anything come and go with it.
    fn keep(&mut self, at: SpawnerAt, pack: Pack, content: &Content, module: &AreaModule) {
        let (count, course) = match module {
            AreaModule::Spawner(spawner) => {
                let course = Course {
                    stage: Stage::Dormant,
                    last: None,
                    spawned: 0,
                    turn: 0,
                    exhausted: vec![false; spawner.populations.len()],
                };
                let finite = spawner
                    .populations
                    .iter()
                    .any(|population| population.size.is_some());
                if finite || spawner.dormant {
                    self.moving.insert(at);
                }
                (0, Some(course))
            }
            _ => (1, None),
        };
        self.kept.insert(
            at,
            Entries {
                pack,
                entries: vec![Entry::Idle; count],
                sign: None,
                course,
            },
        );
        self.empty
            .extend((0..count as u32).map(|entry| (at, entry)));
        let shares: Vec<&SpawnShare> = match module {
            AreaModule::Spawner(spawner) => spawner.shares().collect(),
            _ => Vec::new(),
        };
        let keeps_hours = keeps_hours(content, module);
        if !keeps_hours && shares.iter().all(|share| share.when.is_none()) {
            return;
        }
        if let Err(place) = self.under_sky.binary_search(&at) {
            self.under_sky.insert(place, at);
        }
        for when in shares.iter().filter_map(|share| share.when.as_ref()) {
            if !self.tended.followed.iter().any(|(known, _)| known == when) {
                self.tended.followed.push((when.clone(), false));
            }
        }
        if keeps_hours {
            self.tended.time.get_or_insert(None);
        }
        self.tended.unsettled = true;
    }

    /// The population at its turn in the spawner kept at `at`.
    fn active<'s>(&self, at: SpawnerAt, spawner: &'s Spawner) -> Option<&'s Population> {
        let population = self.kept.get(&at)?.course.as_ref()?.active()?;
        spawner.populations.get(population)
    }

    /// Gives the spawner kept at `at` entries for `count` to stand at once, all of them empty.
    fn resize(&mut self, at: SpawnerAt, count: u32) {
        let Some(kept) = self.kept.get_mut(&at) else {
            return;
        };
        for entry in 0..kept.entries.len() as u32 {
            self.empty.remove(&(at, entry));
        }
        kept.entries = vec![Entry::Idle; count as usize];
        self.empty.extend((0..count).map(|entry| (at, entry)));
    }

    fn set(&mut self, at: SpawnerAt, entry: u32, state: Entry) {
        let kept = self
            .kept
            .get_mut(&at)
            .and_then(|entries| entries.entries.get_mut(entry as usize));
        let Some(kept) = kept else {
            return;
        };
        *kept = state;
        match state {
            Entry::Filled { .. } | Entry::Spent => self.empty.remove(&(at, entry)),
            Entry::Freed { .. } | Entry::Idle => self.empty.insert((at, entry)),
        };
    }

    /// Whether the sky has turned for what is kept since it was last tended, or that left
    /// something to see to.
    fn turned(&self, world: &World) -> bool {
        let tended = &self.tended;
        tended.unsettled
            || tended
                .time
                .as_ref()
                .is_some_and(|time| time.as_ref() != sky::time_of_day(world).map(|time| &time.name))
            || tended
                .followed
                .iter()
                .any(|(when, held)| holds(world, when) != *held)
    }
}

/// Fills the entries of the spawners and residents placed on the tile at `tile`: a spawner's
/// from the first population to take its turn, unless it waits for a script to wake it.
pub fn fill(world: &mut World, tile: TileAt) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let area = area::load(&content, area_id);
    let placed = area::placed(&content, area_id);
    for &index in area.modules_in(tile) {
        let module = &placed[index as usize].module;
        if !matches!(module, AreaModule::Spawner(_) | AreaModule::Resident(_)) {
            continue;
        }
        let pack = npc::next_pack(world);
        let at = SpawnerAt(index);
        world
            .resource_mut::<SpawnerEntries>()
            .keep(at, pack, &content, module);
        match module {
            AreaModule::Spawner(spawner) => {
                let woken = spawner
                    .name
                    .as_ref()
                    .is_some_and(|name| world.resource::<SpawnerEntries>().woken.contains(name));
                if !spawner.dormant || woken {
                    begin(world, &content, area_id, at, &[]);
                }
            }
            _ => spawn_entry(world, &content, area, area_id, at, 0, &[]),
        }
    }
}

/// Wakes every dormant spawner named `name`: its first population takes its turn at once, and
/// one the world reaches later wakes as it is reached.
pub fn wake(world: &mut World, name: &str) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let placed = area::placed(&content, area_id);
    let mut spawners = world.resource_mut::<SpawnerEntries>();
    spawners.woken.insert(name.to_owned());
    let dormant: Vec<SpawnerAt> = spawners
        .kept
        .iter()
        .filter(|(at, kept)| {
            let named = matches!(
                &placed[at.0 as usize].module,
                AreaModule::Spawner(spawner) if spawner.name.as_deref() == Some(name)
            );
            named
                && kept
                    .course
                    .as_ref()
                    .is_some_and(|course| course.stage == Stage::Dormant)
        })
        .map(|(at, _)| *at)
        .collect();
    for at in dormant {
        begin(world, &content, area_id, at, &[]);
    }
}

/// Tends every spawner and home the world has reached: once the sky has turned for them,
/// takes away what it has there no longer and wakes what stood idle under it; then fills again
/// every entry whose wait is over.
pub fn tend(
    world: &mut World,
    characters: &mut QueryState<&'static Position, With<PlayedBy>>,
    sessions: &mut dialogue::Sessions,
) {
    let turned = world.resource::<SpawnerEntries>().turned(world);
    if turned {
        let settled = retire(world, characters, sessions);
        let time = time_now(world);
        let held: Vec<bool> = world
            .resource::<SpawnerEntries>()
            .tended
            .followed
            .iter()
            .map(|(when, _)| holds(world, when))
            .collect();
        let tended = &mut world.resource_mut::<SpawnerEntries>().tended;
        for ((_, was), held) in tended.followed.iter_mut().zip(held) {
            *was = held;
        }
        if let Some(seen) = &mut tended.time {
            *seen = time;
        }
        tended.unsettled = !settled;
    }
    refill(world, characters, turned);
    advance(world, characters);
}

/// Moves every spawner on whose population is exhausted, once all it spawned has fallen, to the
/// next after its delay, or to none once no population is left; and begins every turn that is
/// due.
fn advance(world: &mut World, characters: &mut QueryState<&'static Position, With<PlayedBy>>) {
    let spawners = world.resource::<SpawnerEntries>();
    if spawners.moving.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let placed = area::placed(&content, area_id);
    let now = time::uptime(world);
    let mut exhausted: Vec<SpawnerAt> = Vec::new();
    let mut due: Vec<(SpawnerAt, usize)> = Vec::new();
    for &at in &spawners.moving {
        let (Some(kept), AreaModule::Spawner(spawner)) =
            (spawners.kept.get(&at), &placed[at.0 as usize].module)
        else {
            continue;
        };
        let Some(course) = &kept.course else {
            continue;
        };
        match course.stage {
            Stage::Active(population) => {
                let all_spawned = spawner.populations[population]
                    .size
                    .is_some_and(|size| course.spawned >= size);
                let standing = kept.entries.iter().any(|entry| {
                    matches!(entry, Entry::Filled { stands, .. } if !stat::is_dead(world, *stands))
                });
                if all_spawned && !standing {
                    exhausted.push(at);
                }
            }
            Stage::Waiting { next, at: begins } if now >= begins => due.push((at, next)),
            _ => {}
        }
    }
    if exhausted.is_empty() && due.is_empty() {
        return;
    }
    for at in exhausted {
        let AreaModule::Spawner(spawner) = &placed[at.0 as usize].module else {
            continue;
        };
        let next = world.resource_scope(|world, mut rng: Mut<Rng>| {
            let mut spawners = world.resource_mut::<SpawnerEntries>();
            let course = spawners
                .kept
                .get_mut(&at)
                .and_then(|kept| kept.course.as_mut())
                .expect("a spawner on the move keeps its course");
            let Stage::Active(population) = course.stage else {
                return None;
            };
            course.exhausted[population] = true;
            let next = course.next(spawner, &mut rng);
            course.stage = match next {
                Some(next) => Stage::Waiting {
                    next,
                    at: now + spawner.delay_between,
                },
                None => Stage::Exhausted,
            };
            Some((population, next))
        });
        let Some((population, next)) = next else {
            continue;
        };
        world.resource_mut::<SpawnerEntries>().resize(at, 0);
        tell::<PopulationExhausted>(world, spawner, population);
        if next.is_none() {
            world.resource_mut::<SpawnerEntries>().moving.remove(&at);
            tell::<SpawnerExhausted>(world, spawner, population);
        }
    }
    if due.is_empty() {
        return;
    }
    let watching = watchers(world, characters);
    for (at, population) in due {
        activate(world, &content, area_id, at, population, &watching);
    }
}

/// Has the first population to take a turn in the spawner kept at `at` take it at once.
fn begin(
    world: &mut World,
    content: &Content,
    area_id: AreaId,
    at: SpawnerAt,
    watching: &[Pos<Meters>],
) {
    let AreaModule::Spawner(spawner) = &area::placed(content, area_id)[at.0 as usize].module else {
        return;
    };
    let first = world.resource_scope(|world, mut rng: Mut<Rng>| {
        world
            .resource::<SpawnerEntries>()
            .kept
            .get(&at)
            .and_then(|kept| kept.course.as_ref())
            .and_then(|course| course.next(spawner, &mut rng))
    });
    match first {
        Some(population) => activate(world, content, area_id, at, population, watching),
        None => {
            if let Some(course) = world
                .resource_mut::<SpawnerEntries>()
                .kept
                .get_mut(&at)
                .and_then(|kept| kept.course.as_mut())
            {
                course.stage = Stage::Exhausted;
            }
        }
    }
}

/// Begins the turn of the spawner's `population`: its entries made ready for as many as it keeps
/// standing at once, the area's scripts told, and every entry filled.
fn activate(
    world: &mut World,
    content: &Content,
    area_id: AreaId,
    at: SpawnerAt,
    population: usize,
    watching: &[Pos<Meters>],
) {
    let AreaModule::Spawner(spawner) = &area::placed(content, area_id)[at.0 as usize].module else {
        return;
    };
    let count = spawner.populations[population].max_spawned_at_once;
    let mut spawners = world.resource_mut::<SpawnerEntries>();
    let Some(course) = spawners
        .kept
        .get_mut(&at)
        .and_then(|kept| kept.course.as_mut())
    else {
        return;
    };
    course.stage = Stage::Active(population);
    course.last = Some(population);
    course.spawned = 0;
    course.turn += 1;
    spawners.resize(at, count);
    tell::<PopulationActivated>(world, spawner, population);
    let area = area::load(content, area_id);
    for entry in 0..count {
        spawn_entry(world, content, area, area_id, at, entry, watching);
    }
}

/// Raises a spawner's event for the area's scripts that answer it under the spawner's name, with
/// the population it concerns numbered from 1 and the middle of where it spawns.
fn tell<E: SpawnerEvent>(world: &mut World, spawner: &Spawner, population: usize) {
    let Some(name) = spawner.name.clone() else {
        return;
    };
    let payload = vec![
        Value::Int(population as i64 + 1),
        Value::Point(spawner.within.map(|bounds| bounds.center())),
    ];
    let mut instances = world.resource_mut::<Instances>();
    let kind = EventKind::of::<E>();
    for owner in instances.owners_answering(kind).to_vec() {
        let name = name.clone();
        instances.raise(
            owner,
            kind,
            None,
            payload.clone(),
            move |event: &ScriptEvent| {
                event
                    .downcast_ref::<E>()
                    .is_some_and(|event| event.spawner() == name)
            },
        );
    }
}

/// Takes away what stands of a share that holds no longer under the sky, each once no character
/// has it in sight, its entry to be filled again within its spawner's respawn. A resident that
/// is away leaves at once, but for whoever is in the middle of something with it, and its sign
/// stands where it lives for everyone else until it is back. Says whether that is all seen to:
/// nothing waits to go for somebody to look away or to be done with it.
fn retire(
    world: &mut World,
    characters: &mut QueryState<&'static Position, With<PlayedBy>>,
    sessions: &mut dialogue::Sessions,
) -> bool {
    let under_sky = world.resource::<SpawnerEntries>().under_sky.clone();
    if under_sky.is_empty() {
        return true;
    }
    let content = world.resource::<Content>().clone();
    let placed = area::placed(&content, world.resource::<WorldArea>().0);
    let mut settled = keep_residents(world, placed, &under_sky, sessions);
    let spawners = world.resource::<SpawnerEntries>();
    let mut over: Vec<(SpawnerAt, u32, Entity, Seconds)> = Vec::new();
    for &at in &under_sky {
        let AreaModule::Spawner(spawner) = &placed[at.0 as usize].module else {
            continue;
        };
        let Some(population) = spawners.active(at, spawner) else {
            continue;
        };
        let Some(respawn) = population.respawn else {
            continue;
        };
        for (index, share) in population.spawns.iter().enumerate() {
            if holding(world, share) > 0.0 {
                continue;
            }
            let entries = spawners.kept[&at].entries.iter().enumerate();
            over.extend(entries.filter_map(|(entry, state)| match *state {
                Entry::Filled {
                    share: Some(drawn),
                    stands,
                } if drawn.0 as usize == index && !stat::is_dead(world, stands) => {
                    Some((at, entry as u32, stands, respawn))
                }
                _ => None,
            }));
        }
    }
    if over.is_empty() {
        return settled;
    }
    let watching = watchers(world, characters);
    let sight = visibility::sight(world);
    let now = time::uptime(world);
    for (at, entry, stands, respawn) in over {
        let seen = position(world, stands)
            .is_some_and(|stood| watching.iter().any(|&from| from.distance(stood) <= sight));
        if seen {
            settled = false;
            continue;
        }
        match world.get::<Npc>(stands) {
            Some(_) => npc::despawn(world, stands),
            None => {
                world.despawn(stands);
            }
        }
        let wait = Seconds(world.resource_mut::<Rng>().rand_float() * respawn.0);
        let mut spawners = world.resource_mut::<SpawnerEntries>();
        spawners.set(
            at,
            entry,
            Entry::Freed {
                since: now,
                wait: Some(wait),
            },
        );
        // What the sky took away did not fall: its population still owes it.
        if let Some(course) = spawners
            .kept
            .get_mut(&at)
            .and_then(|kept| kept.course.as_mut())
        {
            course.spawned = course.spawned.saturating_sub(1);
        }
    }
    settled
}

/// Fills again every entry freed longer ago than its wait: its spawner's or resident's respawn
/// time after a fall, less after a turn of the sky, so a spawner turns from one share to the
/// next over its respawn and never at a stroke. What stood idle under the sky is looked at
/// again once that has `turned`. An entry is filled out of every character's sight where the
/// ground allows.
fn refill(
    world: &mut World,
    characters: &mut QueryState<&'static Position, With<PlayedBy>>,
    turned: bool,
) {
    let spawners = world.resource::<SpawnerEntries>();
    if spawners.empty.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let area = area::load(&content, area_id);
    let placed = area::placed(&content, area_id);
    let now = time::uptime(world);
    let mut due: Vec<(SpawnerAt, u32)> = Vec::new();
    let mut woken: Vec<(SpawnerAt, u32, Seconds)> = Vec::new();
    let mut looked: Option<(SpawnerAt, bool)> = None;
    for &(at, entry) in &spawners.empty {
        let module = &placed[at.0 as usize].module;
        let (respawn, eases_in) = match module {
            AreaModule::Spawner(spawner) => match spawners.active(at, spawner) {
                Some(population) => (population.respawn, true),
                None => continue,
            },
            AreaModule::Resident(resident) => (resident.respawn, false),
            _ => continue,
        };
        match (spawners.kept[&at].entries[entry as usize], respawn) {
            (Entry::Freed { since, wait }, respawn)
                if wait.or(respawn).is_some_and(|wait| now - since >= wait)
                    && grounded(area, module) =>
            {
                due.push((at, entry));
            }
            (Entry::Idle, respawn) if turned => {
                let holds = match looked {
                    Some((known, holds)) if known == at => holds,
                    _ => {
                        looked
                            .insert((at, holds_any(world, spawners, at, module)))
                            .1
                    }
                };
                match respawn {
                    _ if !holds => {}
                    Some(respawn) if eases_in => woken.push((at, entry, respawn)),
                    _ => due.push((at, entry)),
                }
            }
            _ => {}
        }
    }
    if due.is_empty() && woken.is_empty() {
        return;
    }
    for (at, entry, respawn) in woken {
        let wait = Seconds(world.resource_mut::<Rng>().rand_float() * respawn.0);
        world.resource_mut::<SpawnerEntries>().set(
            at,
            entry,
            Entry::Freed {
                since: now,
                wait: Some(wait),
            },
        );
    }
    let watching = watchers(world, characters);
    for (at, entry) in due {
        spawn_entry(world, &content, area, area_id, at, entry, &watching);
    }
}

/// Fills the entry, away from where `watching` look from as far as the ground allows, or leaves
/// it idle when nothing of its spawner holds under the sky. An entry whose ground the world
/// doesn't hold waits, freed, until a character comes near enough that it does.
fn spawn_entry(
    world: &mut World,
    content: &Content,
    area: &Area,
    area_id: AreaId,
    at: SpawnerAt,
    entry: u32,
    watching: &[Pos<Meters>],
) {
    let pack = world.resource::<SpawnerEntries>().kept[&at].pack;
    let module = &area::placed(content, area_id)[at.0 as usize].module;
    if !grounded(area, module) {
        let waits = Entry::Freed {
            since: time::uptime(world),
            wait: Some(Seconds(0.0)),
        };
        world.resource_mut::<SpawnerEntries>().set(at, entry, waits);
        return;
    }
    let (stands, share) = match module {
        AreaModule::Spawner(spawner) => {
            let spawners = world.resource::<SpawnerEntries>();
            let Some(population) = spawners.active(at, spawner) else {
                return;
            };
            let spawned = spawners.kept[&at]
                .course
                .as_ref()
                .map_or(0, |course| course.spawned);
            if population.size.is_some_and(|size| spawned >= size) {
                world
                    .resource_mut::<SpawnerEntries>()
                    .set(at, entry, Entry::Spent);
                return;
            }
            let drawn = world.resource_scope(|world, mut rng: Mut<Rng>| {
                population.draw(&mut rng, |share| holding(world, share))
            });
            let stands = drawn.map(|share| {
                let what = share.of(population).what;
                spawn_drawn(world, area, area_id, spawner, what, pack, watching)
            });
            if stands.is_some()
                && let Some(course) = world
                    .resource_mut::<SpawnerEntries>()
                    .kept
                    .get_mut(&at)
                    .and_then(|kept| kept.course.as_mut())
            {
                course.spawned += 1;
            }
            (stands, drawn)
        }
        AreaModule::Resident(resident) => match absence::away(world, resident.npc) {
            Some(_) => (None, None),
            None => (
                Some(npc::settle(world, content, area, area_id, resident, pack)),
                None,
            ),
        },
        _ => return,
    };
    let state = stands.map_or(Entry::Idle, |stands| Entry::Filled { share, stands });
    world.resource_mut::<SpawnerEntries>().set(at, entry, state);
    if let Some(stands) = stands {
        let turn = turn_of(world.resource::<SpawnerEntries>(), at);
        world.entity_mut(stands).insert(Spawned {
            spawner: at,
            entry,
            turn,
        });
    }
}

/// Whether the world holds the ground the module's entries stand on: a spawner's bounds and a
/// resident's home are held only while a character is near.
fn grounded(area: &Area, module: &AreaModule) -> bool {
    match module {
        AreaModule::Spawner(spawner) => spawner.within.is_none_or(|bounds| area.holds(bounds)),
        AreaModule::Resident(resident) => area.holds(Rect::new(resident.at, Size::zero())),
        _ => true,
    }
}

/// How much of `share` holds under the world's sky, 0 to 1: none of an npc that is away.
fn holding(world: &World, share: &SpawnShare) -> f32 {
    match share.what {
        Spawns::Npc { npc } if absence::away(world, npc).is_some() => 0.0,
        _ => sky::share(world, share.when.as_ref()),
    }
}

/// Whether anything `module` keeps standing is there under the world's sky: of a spawner, of
/// the population at its turn.
fn holds_any(world: &World, spawners: &SpawnerEntries, at: SpawnerAt, module: &AreaModule) -> bool {
    match module {
        AreaModule::Spawner(spawner) => spawners.active(at, spawner).is_some_and(|population| {
            population
                .spawns
                .iter()
                .any(|share| holding(world, share) > 0.0)
        }),
        AreaModule::Resident(resident) => absence::away(world, resident.npc).is_none(),
        _ => false,
    }
}

/// Whether any npc `module` keeps standing is away for some of the game's day.
fn keeps_hours(content: &Content, module: &AreaModule) -> bool {
    let absent = |npc: NpcId| npc.get(content).absence().is_some();
    match module {
        AreaModule::Spawner(spawner) => spawner.npcs().any(absent),
        AreaModule::Resident(resident) => absent(resident.npc),
        _ => false,
    }
}

/// Whether anything of what follows `when` is there under the world's sky.
fn holds(world: &World, when: &SkyCondition) -> bool {
    sky::share(world, Some(when)) > 0.0
}

fn time_now(world: &World) -> Option<TimeOfDayName> {
    sky::time_of_day(world).map(|time| time.name.clone())
}

/// Sees every resident of `homes` that is away out of the world, but for whoever is in the
/// middle of something with it, and its sign into it where it lives; and the sign out again once
/// the resident is back. Says whether none of them stays for anyone.
fn keep_residents(
    world: &mut World,
    placed: &[area::Flattened],
    homes: &[SpawnerAt],
    sessions: &mut dialogue::Sessions,
) -> bool {
    let mut settled = true;
    for &at in homes {
        let AreaModule::Resident(resident) = &placed[at.0 as usize].module else {
            continue;
        };
        let away = absence::away(world, resident.npc);
        let sign = world.resource::<SpawnerEntries>().kept[&at].sign;
        if away.is_none() && sign.is_none() {
            continue;
        }
        let body = match world.resource::<SpawnerEntries>().kept[&at].entries[0] {
            Entry::Filled { stands, .. } if !stat::is_dead(world, stands) => Some(stands),
            _ => None,
        };
        let stays = match (away, body) {
            (Some(_), Some(body)) => absence::engaged(world, sessions, body),
            _ => Vec::new(),
        };
        if let Some(body) = body {
            match (away, stays.is_empty()) {
                (Some(_), true) => npc::despawn(world, body),
                (Some(_), false) => {
                    visibility::show_among(world, body, Some(Viewers::Only(stays.clone())));
                }
                (None, _) => visibility::show_among(world, body, None),
            }
        }
        let left = match (sign, away) {
            (Some((sign, span)), Some(now)) if span == now => Some((sign, span)),
            (sign, away) => {
                if let Some((sign, _)) = sign {
                    area::object::remove(world, sign);
                }
                away.and_then(|span| Some((absence::leave_sign(world, resident, span)?, span)))
            }
        };
        settled &= stays.is_empty();
        if let Some((sign, _)) = left {
            let hidden = (!stays.is_empty()).then_some(Viewers::Except(stays));
            visibility::show_among(world, sign, hidden);
        }
        let mut spawners = world.resource_mut::<SpawnerEntries>();
        let entries = spawners
            .kept
            .get_mut(&at)
            .expect("its home keeps the entry");
        entries.sign = left;
        let freed = matches!(entries.entries[0], Entry::Freed { .. });
        if away.is_some() && body.is_some() && freed {
            spawners.set(at, 0, Entry::Idle);
        }
    }
    settled
}

fn watchers(
    world: &World,
    characters: &mut QueryState<&'static Position, With<PlayedBy>>,
) -> Vec<Pos<Meters>> {
    characters
        .iter(world)
        .map(|position| position.pos)
        .collect()
}

fn spawn_drawn(
    world: &mut World,
    area: &Area,
    area_id: AreaId,
    spawner: &Spawner,
    what: Spawns,
    pack: Pack,
    watching: &[Pos<Meters>],
) -> Entity {
    match what {
        Spawns::Npc { npc } => {
            let mesh = area.mesh_for(true, false);
            let at = free_point(world, &mesh, area_id, spawner.within, watching)
                .expect("checked at load: a spawner has ground outside the safe zones");
            let entity = npc::spawn(world, npc, at, area_id, Some(pack));
            match spawner.within {
                Some(bounds) => world.entity_mut(entity).insert((Wild, Roams(bounds))),
                None => world.entity_mut(entity).insert(Wild),
            };
            entity
        }
        Spawns::Resource { resource: def } => match spawner.on {
            SpawnSurface::Ground => {
                let mesh = area.mesh_for(false, false);
                let at = free_point(world, &mesh, area_id, spawner.within, watching)
                    .expect("checked at load: a spawner has ground to spawn on");
                resource::spawn(world, def, at, area_id)
            }
            SpawnSurface::Water => {
                let mesh = area
                    .standing_water()
                    .expect("an area that spawns on water lays its water");
                let at = free_point(world, &mesh, area_id, spawner.within, watching)
                    .expect("checked at load: a spawner on water has water to spawn on");
                let afloat = resource::spawn(world, def, at, area_id);
                world.entity_mut(afloat).insert(ResourceAfloat);
                afloat
            }
        },
    }
}

/// A walkable point within `within`, or anywhere without it: one nobody stands on and none of
/// `watching` has in sight if there is one, or else one nobody stands on.
fn free_point(
    world: &mut World,
    mesh: &NavMesh,
    area: AreaId,
    within: Option<Rect<Meters>>,
    watching: &[Pos<Meters>],
) -> Option<Pos<Meters>> {
    let sight = visibility::sight(world);
    // A character sees a round of ground, so bounds whose every corner one of them sees are
    // seen whole: nowhere in them is hidden, and looking would cost every draw there is.
    let seen_whole = within.is_some_and(|bounds| {
        watching.iter().any(|from| {
            corners(bounds)
                .iter()
                .all(|corner| from.distance(*corner) <= sight)
        })
    });
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        let spots = world.resource::<Spots>();
        let mut draw = |fits: &dyn Fn(Pos<Meters>) -> bool| match within {
            Some(bounds) => mesh.random_point_within(&mut rng, bounds, fits),
            None => mesh.random_point(&mut rng, fits),
        };
        let free = |at| spots.crowding(area, at, None) == 0;
        let unseen = |at: Pos<Meters>| watching.iter().all(|from| from.distance(at) > sight);
        let hidden = (!watching.is_empty() && !seen_whole)
            .then(|| draw(&|at| free(at) && unseen(at)))
            .flatten();
        hidden.or_else(|| draw(&free)).or_else(|| draw(&|_| true))
    })
}

fn free_entry(mut world: DeferredWorld, context: HookContext) {
    let Spawned {
        spawner,
        entry,
        turn,
    } = *world
        .get::<Spawned>(context.entity)
        .expect("the component being removed");
    let now = time::uptime(&world);
    if let Some(mut entries) = world.get_resource_mut::<SpawnerEntries>()
        && turn_of(&entries, spawner) == turn
    {
        entries.set(
            spawner,
            entry,
            Entry::Freed {
                since: now,
                wait: None,
            },
        );
    }
}

/// How many turns the populations of the spawner kept at `at` have taken; none of a resident.
fn turn_of(spawners: &SpawnerEntries, at: SpawnerAt) -> u32 {
    spawners
        .kept
        .get(&at)
        .and_then(|kept| kept.course.as_ref())
        .map_or(0, |course| course.turn)
}

/// What a spawner's event hands its script: the population it concerns, numbered from 1 in the
/// order the spawner lists them, and the middle of where the spawner spawns.
const SPAWNER_FIELDS: &[(&str, ValueType)] =
    &[("population", ValueType::Integer), ("at", ValueType::Point)];

trait SpawnerEvent: EventDef {
    fn spawner(&self) -> &str;
}

macro_rules! spawner_events {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
        pub struct $name {
            pub spawner: String,
        }

        impl EventDef for $name {
            fn owners(&self) -> &'static [RunnerKind] {
                &[AREA_RUNNER]
            }

            fn has_subject(&self) -> bool {
                false
            }

            fn fields(&self) -> &'static [(&'static str, ValueType)] {
                SPAWNER_FIELDS
            }
        }

        impl SpawnerEvent for $name {
            fn spawner(&self) -> &str {
                &self.spawner
            }
        }
    )*};
}

spawner_events! {
    /// One of the populations of the area's spawner named `spawner` began its turn.
    PopulationActivated,
    /// One of the populations of the area's spawner named `spawner` spawned all it spawns, and all
    /// of that has fallen.
    PopulationExhausted,
    /// The area's spawner named `spawner` has no population left to take a turn.
    SpawnerExhausted,
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .effect::<WakeSpawner>("wake_spawner")
        .event::<PopulationActivated>("population_activated")
        .event::<PopulationExhausted>("population_exhausted")
        .event::<SpawnerExhausted>("spawner_exhausted");
}

/// Every spawner an area's script hears must be one the area places.
pub fn check_heard(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    check_heard_of::<PopulationActivated>(loading, &content, scripts);
    check_heard_of::<PopulationExhausted>(loading, &content, scripts);
    check_heard_of::<SpawnerExhausted>(loading, &content, scripts);
}

fn check_heard_of<E: SpawnerEvent>(loading: &mut Loading, content: &Content, scripts: &Scripts) {
    for heard in scripts.hearing::<E>() {
        let area: AreaId = heard.row.id();
        let name = heard.event.spawner();
        let placed = area::placed(content, area).iter().any(|placed| {
            matches!(&placed.module, AreaModule::Spawner(spawner)
                if spawner.name.as_deref() == Some(name))
        });
        if !placed {
            let site = owner_site(content, heard.row);
            loading.json_error(
                &site.file,
                &site.pointer,
                ContentProblem::new(
                    "spawner not here",
                    format!("{} hears {name}", heard.label),
                    "a spawner of that name in the area",
                ),
            );
        }
    }
}
