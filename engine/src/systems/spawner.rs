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
use crate::core::units::{Meters, Seconds};
use crate::systems::WorldArea;
use crate::systems::area::{self, Area, AreaId, AreaModule, Wild};
use crate::systems::movement::{NavMesh, Spots};
use crate::systems::npc::{self, NpcId, Pack, Roams};
use crate::systems::resource::{self, ResourceId};

/// Keeps `count` of what it spawns standing within a rect of the area.
#[derive(JsonSchema, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spawner {
    /// What an entry may be. Every fill draws one, by weight.
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

#[derive(JsonSchema, Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpawnShare {
    #[serde(flatten)]
    pub what: Spawns,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub weight: u32,
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
            spawns: vec![SpawnShare { what, weight: 1 }],
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
        if let Some(respawn) = self.respawn {
            cx.at_least(format!("{pointer}/respawn"), respawn.0, 0.0, "seconds");
        }
    }

    /// One of what it spawns, each as likely as its share of the weights.
    pub fn draw(&self, rng: &mut Rng) -> Spawns {
        let total: u32 = self.spawns.iter().map(|share| share.weight).sum();
        let mut drawn = rng.rand_range(0..total.max(1));
        for share in &self.spawns {
            if drawn < share.weight {
                return share.what;
            }
            drawn -= share.weight;
        }
        self.spawns
            .first()
            .expect("checked at load: a spawner spawns something")
            .what
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
pub struct SpawnerEntries(HashMap<SpawnerAt, Entries>);

struct Entries {
    pack: Pack,
    freed: Vec<Option<Seconds>>,
}

impl SpawnerEntries {
    /// How many of the spawner's entries stand filled.
    pub fn filled(&self, spawner: SpawnerAt) -> usize {
        self.0.get(&spawner).map_or(0, |entries| {
            entries.freed.iter().filter(|freed| freed.is_none()).count()
        })
    }
}

/// Fills the entries of the spawners and residents placed on the tile at `tile`.
pub fn fill(world: &mut World, tile: TileAt) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let area = area::load(&content, area_id);
    let placed = area::placed(&content, area_id);
    for &index in area.modules_in(tile) {
        let count = match &placed[index as usize].module {
            AreaModule::Spawner(spawner) => spawner.count,
            AreaModule::Resident(_) => 1,
            _ => continue,
        };
        let pack = npc::next_pack(world);
        let at = SpawnerAt(index);
        world.resource_mut::<SpawnerEntries>().0.insert(
            at,
            Entries {
                pack,
                freed: vec![None; count as usize],
            },
        );
        for entry in 0..count {
            spawn_entry(world, &content, area, area_id, at, entry);
        }
    }
}

/// Fills again every entry freed longer ago than its spawner's or resident's respawn time.
pub fn refill(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let area_id = world.resource::<WorldArea>().0;
    let placed = area::placed(&content, area_id);
    let now = time::uptime(world);
    let mut due: Vec<(SpawnerAt, u32)> = world
        .resource::<SpawnerEntries>()
        .0
        .iter()
        .filter_map(|(&at, entries)| {
            let respawn = match &placed[at.0 as usize].module {
                AreaModule::Spawner(spawner) => spawner.respawn,
                AreaModule::Resident(resident) => resident.respawn,
                _ => None,
            }?;
            Some((at, entries, respawn))
        })
        .flat_map(|(at, entries, respawn)| {
            entries
                .freed
                .iter()
                .enumerate()
                .filter(move |(_, freed)| freed.is_some_and(|since| now - since >= respawn))
                .map(move |(entry, _)| (at, entry as u32))
        })
        .collect();
    if due.is_empty() {
        return;
    }
    due.sort_unstable();
    let area = area::load(&content, area_id);
    for (at, entry) in due {
        if let Some(entries) = world.resource_mut::<SpawnerEntries>().0.get_mut(&at) {
            entries.freed[entry as usize] = None;
        }
        spawn_entry(world, &content, area, area_id, at, entry);
    }
}

fn spawn_entry(
    world: &mut World,
    content: &Content,
    area: &Area,
    area_id: AreaId,
    at: SpawnerAt,
    entry: u32,
) {
    let pack = world.resource::<SpawnerEntries>().0[&at].pack;
    let spawned = match &area::placed(content, area_id)[at.0 as usize].module {
        AreaModule::Spawner(spawner) => {
            let what = spawner.draw(&mut world.resource_mut::<Rng>());
            spawn_drawn(world, area, area_id, spawner, what, pack)
        }
        AreaModule::Resident(resident) => {
            npc::settle(world, content, area, area_id, resident, pack)
        }
        _ => return,
    };
    world
        .entity_mut(spawned)
        .insert(Spawned { spawner: at, entry });
}

fn spawn_drawn(
    world: &mut World,
    area: &Area,
    area_id: AreaId,
    spawner: &Spawner,
    what: Spawns,
    pack: Pack,
) -> Entity {
    match what {
        Spawns::Npc { npc } => {
            let mesh = area.mesh_for(true, false);
            let at = free_point(world, &mesh, area_id, spawner.within)
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
            let at = free_point(world, &mesh, area_id, spawner.within)
                .expect("checked at load: a spawner has ground to spawn on");
            resource::spawn(world, def, at, area_id)
        }
    }
}

/// A walkable point within `within`, or anywhere without it, that nobody stands on if there is one.
fn free_point(
    world: &mut World,
    mesh: &NavMesh,
    area: AreaId,
    within: Option<Rect<Meters>>,
) -> Option<Pos<Meters>> {
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        let spots = world.resource::<Spots>();
        let mut draw = |fits: &dyn Fn(Pos<Meters>) -> bool| match within {
            Some(bounds) => mesh.random_point_within(&mut rng, bounds, fits),
            None => mesh.random_point(&mut rng, fits),
        };
        draw(&|at| spots.crowding(area, at, None) == 0).or_else(|| draw(&|_| true))
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
    if let Some(freed) = entries
        .0
        .get_mut(&spawner)
        .and_then(|entries| entries.freed.get_mut(entry as usize))
    {
        *freed = Some(now);
    }
}
