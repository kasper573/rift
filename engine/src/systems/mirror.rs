//! What a world holds of its neighbours' entities: a ghost of each that stands within the halo
//! around its cell, carrying the components its owner replicates and those the server reads of
//! its neighbours, refreshed every tick. A ghost is seen, scanned and replicated like anything
//! else, and played by nobody here: whatever acts on it goes to its owner.

use std::any::Any;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::{ComponentTicks, Tick};
use bevy_ecs::entity::{EntityHashMap, EntityMapper};
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_replicon::prelude::{AppRuleExt, Replicated};
use bevy_replicon::shared::replication::registry::receive_fns::MutWrite;
use bevy_replicon::shared::replication::rules::filter::FilterRules;
use rayon::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::units::Meters;
use crate::systems::identity::{GlobalId, Named};
use crate::systems::movement::Position;
use crate::systems::visibility;
use crate::systems::worlds::Worlds;

/// How far past its cell's edge a world sees: at least as far as a client sees, and as far as any
/// script or skill reaches.
pub const HALO: Meters = Meters(32.0);

/// Stands in for an entity the world at `owner` plays.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ghost {
    pub owner: usize,
}

pub fn is_ghost(world: &World, entity: Entity) -> bool {
    world.get::<Ghost>(entity).is_some()
}

/// How each system declares what of its entities leaves their world: to the clients that see them,
/// and to the neighbouring cells that hold their ghosts. Everything a client is shown of an entity
/// it doesn't own is mirrored too, so a ghost shows as its owner does.
pub trait AppMirrorExt {
    /// Replicated to clients and mirrored into ghosts.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone;

    /// Replicated once to clients, and mirrored into ghosts.
    fn replicate_once<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone;

    /// Replicated to clients where `F` holds, and mirrored into ghosts.
    fn replicate_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone;

    /// Told to an entity's own client alone, which no neighbour reads.
    fn replicate_private<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned;

    /// Told to an entity's own client alone, where `F` holds.
    fn replicate_private_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned;

    /// Mirrored into ghosts without reaching clients: what the server reads of its neighbours'
    /// entities.
    fn mirror<C: Component + Clone>(&mut self) -> &mut Self;
}

impl AppMirrorExt for App {
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone,
    {
        AppRuleExt::replicate::<C>(self);
        self.mirror::<C>()
    }

    fn replicate_once<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone,
    {
        AppRuleExt::replicate_once::<C>(self);
        self.mirror::<C>()
    }

    fn replicate_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned + Clone,
    {
        AppRuleExt::replicate_filtered::<C, F>(self);
        self.mirror::<C>()
    }

    fn replicate_private<C>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        AppRuleExt::replicate::<C>(self);
        visibility::keep_private::<C>(self);
        self
    }

    fn replicate_private_filtered<C, F: FilterRules>(&mut self) -> &mut Self
    where
        C: Component<Mutability: MutWrite<C>> + Serialize + DeserializeOwned,
    {
        AppRuleExt::replicate_filtered::<C, F>(self);
        visibility::keep_private::<C>(self);
        self
    }

    fn mirror<C: Component + Clone>(&mut self) -> &mut Self {
        self.world_mut()
            .get_resource_or_init::<Mirrors>()
            .0
            .push(MirrorFns {
                changed: |world, entity, since| {
                    let entity = world.entity(entity);
                    let component = entity.get::<C>()?;
                    let changed = since.is_none_or(|(last_run, this_run)| {
                        entity
                            .get_change_ticks::<C>()
                            .is_some_and(|ticks: ComponentTicks| {
                                ticks.is_changed(last_run, this_run)
                            })
                    });
                    changed.then(|| Box::new(component.clone()) as Part)
                },
                apply: |world, entity, part, named| {
                    let mut component = *part.downcast::<C>().expect("the part registered as C");
                    C::map_entities(&mut component, named);
                    world.entity_mut(entity).insert(component);
                },
            });
        self
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Mirrors>();
    app.init_resource::<Exchanged>();
    app.world_mut().register_component::<Ghost>();
}

/// Hands every world the ghosts of what its neighbours own within its halo: first who stands
/// there, so each world spawns the ghosts it lacks and drops those gone, then what changed of each.
pub fn exchange(worlds: &Worlds, apps: &mut [&mut World]) {
    let count = apps.len();
    if !worlds.has_borders() {
        return;
    }
    let standing: Vec<Standing> = apps
        .par_iter()
        .enumerate()
        .map(|(index, world)| standing_in_halos(worlds, index, world))
        .collect();
    let mut arriving: Vec<Standing> = vec![Vec::new(); count];
    for (owner, halos) in standing.iter().enumerate() {
        for (neighbour, held) in halos {
            arriving[*neighbour].push((owner, held.clone()));
        }
    }
    let fresh: Vec<Vec<(usize, Vec<GlobalId>)>> = apps
        .par_iter_mut()
        .zip(arriving)
        .map(|(world, arriving)| settle_ghosts(world, arriving))
        .collect();
    let mut new_for: Vec<Vec<(usize, Vec<GlobalId>)>> = vec![Vec::new(); count];
    for (neighbour, spawned) in fresh.into_iter().enumerate() {
        for (owner, ids) in spawned {
            new_for[owner].push((neighbour, ids));
        }
    }
    let views: Vec<&World> = apps.iter().map(|world| &**world).collect();
    let updates: Vec<Vec<(usize, Halo)>> = views
        .par_iter()
        .zip(&standing)
        .zip(&new_for)
        .map(|((world, halos), new_for)| gather(&views, world, halos, new_for))
        .collect();
    let mut incoming: Vec<Vec<Halo>> = (0..count).map(|_| Vec::new()).collect();
    for halos in updates {
        for (neighbour, halo) in halos {
            incoming[neighbour].push(halo);
        }
    }
    apps.par_iter_mut()
        .zip(incoming)
        .for_each(|(world, incoming)| apply(world, incoming));
    for world in apps.iter_mut() {
        let tick = world.change_tick();
        world.resource_mut::<Exchanged>().0 = Some(tick);
    }
}

type Part = Box<dyn Any + Send + Sync>;

/// The ticks a change is looked for between, or none to take a component as it stands.
type Since = Option<(Tick, Tick)>;

struct MirrorFns {
    changed: fn(&World, Entity, Since) -> Option<Part>,
    apply: fn(&mut World, Entity, Part, &mut Across),
}

/// The mirrored components, in the order every world registers them.
#[derive(Resource, Default)]
struct Mirrors(Vec<MirrorFns>);

/// The tick a world last handed its ghosts out at, so the next hands out only what changed since.
#[derive(Resource, Default)]
struct Exchanged(Option<Tick>);

/// Maps the owner's entities to this world's: what the owner names, this world holds under the
/// same name, or not at all.
struct Across {
    owner: Arc<EntityHashMap<GlobalId>>,
    here: Arc<HashMap<GlobalId, Entity>>,
}

impl EntityMapper for Across {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        self.owner
            .get(&source)
            .and_then(|id| self.here.get(id))
            .copied()
            .unwrap_or(Entity::PLACEHOLDER)
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

/// What an owner hands a neighbour: each ghost's changed parts, and the owner's names for its
/// entities, which the parts refer to.
struct Halo {
    owner_names: Arc<EntityHashMap<GlobalId>>,
    ghosts: Vec<GhostUpdate>,
}

struct GhostUpdate {
    entity: Entity,
    parts: Vec<(usize, Part)>,
}

/// Who a world owns within each neighbour's halo, by neighbour.
type Standing = Vec<(usize, Vec<(Entity, GlobalId)>)>;

fn standing_in_halos(worlds: &Worlds, index: usize, world: &World) -> Standing {
    let mut halos: HashMap<usize, Vec<(Entity, GlobalId)>> = HashMap::default();
    let mut owned = world
        .try_query_filtered::<(Entity, &GlobalId, &Position), (With<Replicated>, Without<Ghost>)>()
        .expect("GlobalId, Position, Replicated and Ghost are registered in every world");
    for (entity, &id, position) in owned.iter(world) {
        for neighbour in worlds.around(index, position.pos, HALO) {
            halos.entry(neighbour).or_default().push((entity, id));
        }
    }
    halos.into_iter().collect()
}

/// Spawns a ghost for every named entity the world lacks and despawns those their owner no longer
/// lists; the names spawned, by owner.
fn settle_ghosts(world: &mut World, arriving: Standing) -> Vec<(usize, Vec<GlobalId>)> {
    let mut spawned = Vec::new();
    let mut listed: HashMap<usize, Vec<GlobalId>> = HashMap::default();
    for (owner, held) in arriving {
        let mut fresh = Vec::new();
        for (_, id) in &held {
            if !world.resource::<Named>().contains(*id) {
                world.spawn((Ghost { owner }, *id, Replicated));
                fresh.push(*id);
            }
        }
        listed.insert(owner, held.into_iter().map(|(_, id)| id).collect());
        spawned.push((owner, fresh));
    }
    let gone: Vec<Entity> = world
        .query::<(Entity, &Ghost, &GlobalId)>()
        .iter(world)
        .filter(|(_, ghost, id)| listed.get(&ghost.owner).is_none_or(|ids| !ids.contains(id)))
        .map(|(entity, _, _)| entity)
        .collect();
    for entity in gone {
        world.despawn(entity);
    }
    spawned
}

fn gather(
    views: &[&World],
    world: &World,
    halos: &Standing,
    new_for: &[(usize, Vec<GlobalId>)],
) -> Vec<(usize, Halo)> {
    let mirrors = world.resource::<Mirrors>();
    let since = world
        .resource::<Exchanged>()
        .0
        .map(|last| (last, world.read_change_tick()));
    let owner_names = world.resource::<Named>().by_entity();
    halos
        .iter()
        .map(|(neighbour, held)| {
            let fresh = new_for
                .iter()
                .find(|(other, _)| other == neighbour)
                .map(|(_, ids)| ids.as_slice())
                .unwrap_or(&[]);
            let there = views[*neighbour].resource::<Named>();
            let ghosts = held
                .iter()
                .filter_map(|&(entity, id)| {
                    let whole = fresh.contains(&id);
                    let parts: Vec<(usize, Part)> = mirrors
                        .0
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, fns)| {
                            (fns.changed)(world, entity, if whole { None } else { since })
                                .map(|part| (slot, part))
                        })
                        .collect();
                    let ghost = there.get(id)?;
                    (!parts.is_empty()).then_some(GhostUpdate {
                        entity: ghost,
                        parts,
                    })
                })
                .collect();
            (
                *neighbour,
                Halo {
                    owner_names: owner_names.clone(),
                    ghosts,
                },
            )
        })
        .collect()
}

fn apply(world: &mut World, incoming: Vec<Halo>) {
    let here = world.resource::<Named>().by_name();
    world.resource_scope(|world, mirrors: Mut<Mirrors>| {
        for halo in incoming {
            let mut across = Across {
                owner: halo.owner_names,
                here: here.clone(),
            };
            for update in halo.ghosts {
                for (slot, part) in update.parts {
                    (mirrors.0[slot].apply)(world, update.entity, part, &mut across);
                }
            }
        }
    });
}
