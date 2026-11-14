use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, RowCheck, SchemaMark, serial};
use crate::core::ground::TileAt;
use crate::core::math::{Pos, Rect, Rng};
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::area::{self, Area, AreaId, AreaModule, Wild};
use crate::systems::movement::{NavMesh, Position, Spots};
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
    /// Those of them whose entries come and go with the sky, in their order in the scene.
    under_sky: Vec<SpawnerAt>,
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
    /// Standing, as the share it was drawn from; a resident is of none.
    Filled(Option<ShareAt>),
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
                .filter(|entry| matches!(entry, Entry::Filled(_)))
                .count()
        })
    }

    fn keep(&mut self, at: SpawnerAt, entries: Entries, under_sky: bool) {
        self.kept.insert(at, entries);
        if under_sky && let Err(place) = self.under_sky.binary_search(&at) {
            self.under_sky.insert(place, at);
        }
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
        world.resource_mut::<SpawnerEntries>().keep(
            at,
            Entries {
                pack,
                entries: vec![Entry::Idle; count as usize],
                sign: None,
            },
            comes_and_goes(&content, module),
        );
        for entry in 0..count {
            spawn_entry(world, &content, area, area_id, at, entry, &[]);
        }
    }
}

/// Takes away what stands of a share that holds no longer under the sky, each once no character
/// has it in sight, its entry to be filled again within its spawner's respawn. A resident that
/// is away leaves at once, but for whoever is in the middle of something with it, and its sign
/// stands where it lives for everyone else until it is back.
pub fn retire(
    world: &mut World,
    spawned: &mut QueryState<(Entity, &'static Spawned, &'static Position)>,
    characters: &mut QueryState<&'static Position, With<PlayedBy>>,
    sessions: &mut dialogue::Sessions,
) {
    let under_sky = world.resource::<SpawnerEntries>().under_sky.clone();
    if under_sky.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let placed = area::placed(&content, world.resource::<WorldArea>().0);
    let standing: Vec<(Entity, Spawned, Pos<Meters>)> = spawned
        .iter(world)
        .filter(|(_, spawned, _)| under_sky.binary_search(&spawned.spawner).is_ok())
        .map(|(entity, &spawned, position)| (entity, spawned, position.pos))
        .collect();
    keep_residents(world, placed, &under_sky, &standing, sessions);
    let entries = world.resource::<SpawnerEntries>();
    let over: Vec<(Entity, Spawned, Pos<Meters>, Seconds)> = standing
        .into_iter()
        .filter_map(|(entity, spawned, at)| {
            let AreaModule::Spawner(spawner) = &placed[spawned.spawner.0 as usize].module else {
                return None;
            };
            let Entry::Filled(Some(share)) =
                entries.kept.get(&spawned.spawner)?.entries[spawned.entry as usize]
            else {
                return None;
            };
            let over = holding(world, share.of(spawner)) <= 0.0 && !stat::is_dead(world, entity);
            over.then_some((entity, spawned, at, spawner.respawn?))
        })
        .collect();
    if over.is_empty() {
        return;
    }
    let watching = watchers(world, characters);
    let sight = visibility::sight(world);
    let now = time::uptime(world);
    for (entity, spawned, at, respawn) in over {
        if watching.iter().any(|&from| from.distance(at) <= sight) {
            continue;
        }
        match world.get::<Npc>(entity) {
            Some(_) => npc::despawn(world, entity),
            None => {
                world.despawn(entity);
            }
        }
        let wait = Seconds(world.resource_mut::<Rng>().rand_float() * respawn.0);
        world
            .resource_mut::<SpawnerEntries>()
            .kept
            .get_mut(&spawned.spawner)
            .expect("its spawner keeps the entry")
            .entries[spawned.entry as usize] = Entry::Freed {
            since: now,
            wait: Some(wait),
        };
    }
}

/// Fills again every entry freed longer ago than its wait: its spawner's or resident's respawn
/// time after a fall, less after a turn of the sky, so a spawner turns from one share to the
/// next over its respawn and never at a stroke. An entry is filled out of every character's
/// sight where the ground allows.
pub fn refill(world: &mut World, characters: &mut QueryState<&'static Position, With<PlayedBy>>) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let placed = area::placed(&content, area_id);
    let now = time::uptime(world);
    let mut due: Vec<(SpawnerAt, u32)> = Vec::new();
    let mut woken: Vec<(SpawnerAt, u32, Seconds)> = Vec::new();
    for (&at, entries) in &world.resource::<SpawnerEntries>().kept {
        let module = &placed[at.0 as usize].module;
        let (respawn, eases_in) = match module {
            AreaModule::Spawner(spawner) => (spawner.respawn, true),
            AreaModule::Resident(resident) => (resident.respawn, false),
            _ => continue,
        };
        let mut holds = None;
        for (entry, state) in entries.entries.iter().enumerate() {
            match (*state, respawn) {
                (Entry::Freed { since, wait }, Some(respawn))
                    if now - since >= wait.unwrap_or(respawn) =>
                {
                    due.push((at, entry as u32));
                }
                (Entry::Idle, respawn)
                    if *holds.get_or_insert_with(|| holds_any(world, module)) =>
                {
                    match respawn {
                        Some(respawn) if eases_in => woken.push((at, entry as u32, respawn)),
                        _ => due.push((at, entry as u32)),
                    }
                }
                _ => {}
            }
        }
    }
    if due.is_empty() && woken.is_empty() {
        return;
    }
    woken.sort_unstable_by_key(|&(at, entry, _)| (at, entry));
    for (at, entry, respawn) in woken {
        let wait = Seconds(world.resource_mut::<Rng>().rand_float() * respawn.0);
        if let Some(entries) = world.resource_mut::<SpawnerEntries>().kept.get_mut(&at) {
            entries.entries[entry as usize] = Entry::Freed {
                since: now,
                wait: Some(wait),
            };
        }
    }
    due.sort_unstable();
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
    let (spawned, state) = match &area::placed(content, area_id)[at.0 as usize].module {
        AreaModule::Spawner(spawner) => {
            let drawn = world.resource_scope(|world, mut rng: Mut<Rng>| {
                spawner.draw(&mut rng, |share| holding(world, share))
            });
            let spawned = drawn.map(|share| {
                let what = share.of(spawner).what;
                spawn_drawn(world, area, area_id, spawner, what, pack, watching)
            });
            (
                spawned,
                drawn.map_or(Entry::Idle, |share| Entry::Filled(Some(share))),
            )
        }
        AreaModule::Resident(resident) => match absence::away(world, resident.npc) {
            Some(_) => (None, Entry::Idle),
            None => (
                Some(npc::settle(world, content, area, area_id, resident, pack)),
                Entry::Filled(None),
            ),
        },
        _ => return,
    };
    world
        .resource_mut::<SpawnerEntries>()
        .kept
        .get_mut(&at)
        .expect("its spawner keeps the entry")
        .entries[entry as usize] = state;
    if let Some(spawned) = spawned {
        world
            .entity_mut(spawned)
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

/// Whether what `module` keeps standing comes and goes with the sky: by the `when` of a share,
/// or the absence of an npc.
fn comes_and_goes(content: &Content, module: &AreaModule) -> bool {
    let absent = |npc: NpcId| npc.get(content).absence().is_some();
    match module {
        AreaModule::Spawner(spawner) => spawner.spawns.iter().any(|share| {
            share.when.is_some() || matches!(share.what, Spawns::Npc { npc } if absent(npc))
        }),
        AreaModule::Resident(resident) => absent(resident.npc),
        _ => false,
    }
}

/// Sees every resident of `homes` that is away out of the world, but for whoever is in the
/// middle of something with it, and its sign into it where it lives; and the sign out again once
/// the resident is back.
fn keep_residents(
    world: &mut World,
    placed: &[area::Flattened],
    homes: &[SpawnerAt],
    standing: &[(Entity, Spawned, Pos<Meters>)],
    sessions: &mut dialogue::Sessions,
) {
    for &at in homes {
        let AreaModule::Resident(resident) = &placed[at.0 as usize].module else {
            continue;
        };
        let away = absence::away(world, resident.npc);
        let sign = world.resource::<SpawnerEntries>().kept[&at].sign;
        if away.is_none() && sign.is_none() {
            continue;
        }
        let body = standing
            .iter()
            .find(|(_, spawned, _)| spawned.spawner == at)
            .map(|&(body, ..)| body)
            .filter(|&body| !stat::is_dead(world, body));
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
        if let Some((sign, _)) = left {
            let hidden = (!stays.is_empty()).then_some(Viewers::Except(stays));
            visibility::show_among(world, sign, hidden);
        }
        let mut entries = world.resource_mut::<SpawnerEntries>();
        let entries = entries.kept.get_mut(&at).expect("its home keeps the entry");
        entries.sign = left;
        if away.is_some() && body.is_some() && matches!(entries.entries[0], Entry::Freed { .. }) {
            entries.entries[0] = Entry::Idle;
        }
    }
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
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        let spots = world.resource::<Spots>();
        let mut draw = |fits: &dyn Fn(Pos<Meters>) -> bool| match within {
            Some(bounds) => mesh.random_point_within(&mut rng, bounds, fits),
            None => mesh.random_point(&mut rng, fits),
        };
        let free = |at| spots.crowding(area, at, None) == 0;
        let unseen = |at: Pos<Meters>| watching.iter().all(|from| from.distance(at) > sight);
        let hidden = (!watching.is_empty())
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
    let Some(mut entries) = world.get_resource_mut::<SpawnerEntries>() else {
        return;
    };
    if let Some(state) = entries
        .kept
        .get_mut(&spawner)
        .and_then(|entries| entries.entries.get_mut(entry as usize))
    {
        *state = Entry::Freed {
            since: now,
            wait: None,
        };
    }
}
