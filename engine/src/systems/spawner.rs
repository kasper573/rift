use std::collections::BTreeSet;

use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, RowCheck, SchemaMark, serial};
use crate::core::geometry::corners;
use crate::core::ground::TileAt;
use crate::core::math::{Pos, Rect, Rng};
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::area::{self, Area, AreaId, AreaModule, TimeOfDayName, Wild};
use crate::systems::movement::{NavMesh, Position, Spots, position};
use crate::systems::npc::{self, AbsenceSpanAt, Npc, NpcId, Pack, Roams, absence};
use crate::systems::player::PlayedBy;
use crate::systems::resource::{self, ResourceId};
use crate::systems::script::{SkyCondition, sky};
use crate::systems::visibility::Viewers;
use crate::systems::{WorldArea, dialogue, stat, visibility};

/// Keeps `count` of what it spawns standing within a rect of the area.
#[derive(JsonSchema, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spawner {
    /// What an entry may be. Every fill draws one, by weight, of those whose `when` holds.
    pub spawns: Vec<SpawnShare>,
    #[schemars(transform = SchemaMark::Count.with(true))]
    pub count: u32,
    /// How long a freed entry stays empty. Without one it is never filled again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respawn: Option<Seconds>,
    /// The whole area without one.
    #[schemars(with = "Option<serial::RectRow>")]
    #[serde(
        default,
        serialize_with = "serial::opt_rect",
        deserialize_with = "serial::opt_rect_from",
        skip_serializing_if = "Option::is_none"
    )]
    pub within: Option<Rect<Meters>>,
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
    pub fn of(what: Spawns, within: Option<Rect<Meters>>) -> Spawner {
        Spawner {
            spawns: vec![SpawnShare::of(what)],
            count: 1,
            respawn: None,
            within,
        }
    }

    pub fn npcs(&self) -> impl Iterator<Item = NpcId> + '_ {
        self.spawns.iter().filter_map(|share| match share.what {
            Spawns::Npc { npc } => Some(npc),
            Spawns::Resource { .. } => None,
        })
    }

    pub fn resources(&self) -> impl Iterator<Item = ResourceId> + '_ {
        self.spawns.iter().filter_map(|share| match share.what {
            Spawns::Resource { resource } => Some(resource),
            Spawns::Npc { .. } => None,
        })
    }

    /// Whether what it spawns keeps out of the safe zones, as wild npcs do.
    pub fn keeps_wild(&self) -> bool {
        self.npcs().next().is_some()
    }

    /// The files it spawns, as a problem names them.
    pub fn named(&self, content: &Content) -> String {
        let paths: Vec<String> = self
            .spawns
            .iter()
            .map(|share| share.what.path(content).to_owned())
            .collect();
        ui::words::join_and(&paths)
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.at_least(format!("{pointer}/count"), self.count, 1, "entries");
        cx.at_least(
            format!("{pointer}/spawns"),
            self.spawns.len(),
            1,
            "things to spawn",
        );
        for (index, share) in self.spawns.iter().enumerate() {
            cx.at_least(
                format!("{pointer}/spawns/{index}/weight"),
                share.weight,
                1,
                "shares",
            );
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
    pub fn of(self, spawner: &Spawner) -> &SpawnShare {
        &spawner.spawns[self.0 as usize]
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
}

#[derive(Clone, Copy)]
enum Entry {
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

    /// Keeps the entries of the module at `at`, all of them empty, and looks to the sky for
    /// them when the module has anything come and go with it.
    fn keep(&mut self, at: SpawnerAt, pack: Pack, content: &Content, module: &AreaModule) {
        let count = match module {
            AreaModule::Spawner(spawner) => spawner.count,
            _ => 1,
        };
        self.kept.insert(
            at,
            Entries {
                pack,
                entries: vec![Entry::Idle; count as usize],
                sign: None,
            },
        );
        self.empty.extend((0..count).map(|entry| (at, entry)));
        let shares: &[SpawnShare] = match module {
            AreaModule::Spawner(spawner) => &spawner.spawns,
            _ => &[],
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
            Entry::Filled { .. } => self.empty.remove(&(at, entry)),
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

/// Fills the entries of the spawners and residents placed on the tile at `tile`.
pub fn fill(world: &mut World, tile: TileAt) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let area = area::load(&content, area_id);
    let placed = area::placed(&content, area_id);
    for &index in area.modules_in(tile) {
        let module = &placed[index as usize].module;
        let count = match module {
            AreaModule::Spawner(spawner) => spawner.count,
            AreaModule::Resident(_) => 1,
            _ => continue,
        };
        let pack = npc::next_pack(world);
        let at = SpawnerAt(index);
        world
            .resource_mut::<SpawnerEntries>()
            .keep(at, pack, &content, module);
        for entry in 0..count {
            spawn_entry(world, &content, area, area_id, at, entry, &[]);
        }
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
        let Some(respawn) = spawner.respawn else {
            continue;
        };
        for (index, share) in spawner.spawns.iter().enumerate() {
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
        world.resource_mut::<SpawnerEntries>().set(
            at,
            entry,
            Entry::Freed {
                since: now,
                wait: Some(wait),
            },
        );
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
    let placed = area::placed(&content, area_id);
    let now = time::uptime(world);
    let mut due: Vec<(SpawnerAt, u32)> = Vec::new();
    let mut woken: Vec<(SpawnerAt, u32, Seconds)> = Vec::new();
    let mut looked: Option<(SpawnerAt, bool)> = None;
    for &(at, entry) in &spawners.empty {
        let module = &placed[at.0 as usize].module;
        let (respawn, eases_in) = match module {
            AreaModule::Spawner(spawner) => (spawner.respawn, true),
            AreaModule::Resident(resident) => (resident.respawn, false),
            _ => continue,
        };
        match (spawners.kept[&at].entries[entry as usize], respawn) {
            (Entry::Freed { since, wait }, Some(respawn))
                if now - since >= wait.unwrap_or(respawn) =>
            {
                due.push((at, entry));
            }
            (Entry::Idle, respawn) if turned => {
                let holds = match looked {
                    Some((known, holds)) if known == at => holds,
                    _ => looked.insert((at, holds_any(world, module))).1,
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
    let area = area::load(&content, area_id);
    let watching = watchers(world, characters);
    for (at, entry) in due {
        spawn_entry(world, &content, area, area_id, at, entry, &watching);
    }
}

/// Fills the entry, away from where `watching` look from as far as the ground allows, or leaves
/// it idle when nothing of its spawner holds under the sky.
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
    let (stands, share) = match &area::placed(content, area_id)[at.0 as usize].module {
        AreaModule::Spawner(spawner) => {
            let drawn = world.resource_scope(|world, mut rng: Mut<Rng>| {
                spawner.draw(&mut rng, |share| holding(world, share))
            });
            let stands = drawn.map(|share| {
                let what = share.of(spawner).what;
                spawn_drawn(world, area, area_id, spawner, what, pack, watching)
            });
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
        world
            .entity_mut(stands)
            .insert(Spawned { spawner: at, entry });
    }
}

/// How much of `share` holds under the world's sky, 0 to 1: none of an npc that is away.
fn holding(world: &World, share: &SpawnShare) -> f32 {
    match share.what {
        Spawns::Npc { npc } if absence::away(world, npc).is_some() => 0.0,
        _ => sky::share(world, share.when.as_ref()),
    }
}

/// Whether anything `module` keeps standing is there under the world's sky.
fn holds_any(world: &World, module: &AreaModule) -> bool {
    match module {
        AreaModule::Spawner(spawner) => spawner
            .spawns
            .iter()
            .any(|share| holding(world, share) > 0.0),
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
        Spawns::Resource { resource: def } => {
            let mesh = area.mesh_for(false, false);
            let at = free_point(world, &mesh, area_id, spawner.within, watching)
                .expect("checked at load: a spawner has ground to spawn on");
            resource::spawn(world, def, at, area_id)
        }
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
    let Spawned { spawner, entry } = *world
        .get::<Spawned>(context.entity)
        .expect("the component being removed");
    let now = time::uptime(&world);
    if let Some(mut entries) = world.get_resource_mut::<SpawnerEntries>() {
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
