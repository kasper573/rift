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

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::handoff::Pulled;
use crate::systems::identity::{GlobalId, Named};
use crate::systems::movement::Position;
use crate::systems::npc::NpcId;
use crate::systems::player::{ClientId, PlayedBy};
use crate::systems::rewards::KillCredited;
use crate::systems::script::event::RiftEvent;
use crate::systems::script::{RiftScripts, owner_site};
use crate::systems::skill::SkillDef;
use crate::systems::status::StatusEffectId;
use crate::systems::worlds::Worlds;
use crate::systems::{class, combat, stat, status, visibility};

/// Stands in for an entity the world at `owner` plays.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ghost {
    pub owner: usize,
}

pub fn is_ghost(world: &World, entity: Entity) -> bool {
    world.get::<Ghost>(entity).is_some()
}

/// How far past its cell's edge a world sees, and what reaches that far: as far as a client sees,
/// a skill lands or a script's event listens, so nothing acts on what it can't hold. A script's
/// scan, whose distance is an expression, reaches at most this far past its cell.
pub fn halo(content: &Content) -> (Meters, String) {
    let mut floor = (visibility::VIEW_DISTANCE, "a client sees".to_owned());
    let mut raise = |reach: Meters, by: String| {
        if reach > floor.0 {
            floor = (reach, by);
        }
    };
    for (id, skill) in content.table::<SkillDef>().iter() {
        let reach = skill.range.map_or(0.0, |range| range.0)
            + skill.area.as_ref().map_or(0.0, |area| area.reach().0);
        raise(Meters(reach), format!("{} reaches", id.path(content)));
    }
    for (row, procedures) in content.built::<RiftScripts>().owners() {
        for procedure in procedures {
            for (_, event) in &procedure.entries {
                if let RiftEvent::CameNear { within, .. } | RiftEvent::Heard { within, .. } = event
                {
                    raise(
                        *within,
                        format!(
                            "{} listens in {}",
                            procedure.label,
                            owner_site(content, *row).file
                        ),
                    );
                }
            }
        }
    }
    floor
}

/// What lands on an actor from another world: dealt to its ghost here, applied by its owner on
/// the next tick as if dealt there.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Inflict {
        damage: f32,
    },
    Heal {
        amount: f32,
    },
    Status {
        effect: StatusEffectId,
        power: f32,
        lasts: Option<Seconds>,
    },
    /// Asks the owner to hand the target over to the world that posts, where someone in reach
    /// of its ghost wants to deal with it.
    Pull,
    Xp {
        amount: u32,
    },
    /// The target's character is credited with a kill of `npc` that fell at `at`.
    Kill {
        npc: NpcId,
        at: Pos<Meters>,
    },
}

/// Posts `effect` to the owner of `target` when `target` is a ghost here, from `by` as the owner
/// knows it; false when `target` is this world's own to act on.
pub fn post(world: &mut World, target: Entity, by: Option<Entity>, effect: Effect) -> bool {
    let Some(ghost) = world.get::<Ghost>(target).copied() else {
        return false;
    };
    let named = world.resource::<Named>();
    let Some(to) = named.name(target) else {
        return true;
    };
    let letter = Letter {
        to,
        by: by.and_then(|by| named.name(by)),
        from: None,
        effect,
    };
    world.resource_mut::<Outbox>().0.push((ghost.owner, letter));
    true
}

/// The ghost of `client`'s character, when its owner is a neighbour and it stands within reach.
pub fn ghost_played_by(world: &World, client: ClientId) -> Option<Entity> {
    world
        .try_query_filtered::<(Entity, &PlayedBy), With<Ghost>>()?
        .iter(world)
        .find(|(_, played)| played.client == client)
        .map(|(entity, _)| entity)
}

/// Lands what neighbours posted to this world's own entities since the last exchange.
pub fn deliver(world: &mut World) {
    let letters = std::mem::take(&mut world.resource_mut::<Inbox>().0);
    for letter in letters {
        let named = world.resource::<Named>();
        let Some(to) = named.get(letter.to) else {
            continue;
        };
        let by = letter.by.and_then(|by| named.get(by));
        match letter.effect {
            Effect::Inflict { damage } => combat::inflict(world, to, by, damage),
            Effect::Heal { amount } => stat::heal(world, to, amount),
            Effect::Status {
                effect,
                power,
                lasts,
            } => status::apply(world, to, effect, power, lasts),
            Effect::Pull => {
                if let Some(from) = letter.from {
                    world.entity_mut(to).insert(Pulled(from));
                }
            }
            Effect::Xp { amount } => class::grant_xp(world, to, amount),
            Effect::Kill { npc, at } => {
                world.write_message(KillCredited {
                    npc,
                    at,
                    credited: to,
                });
            }
        }
    }
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
        add::<C>(self.world_mut());
        self
    }
}

pub fn register(app: &mut App) {
    prepare(app.world_mut());
}

/// Readies a world to hold ghosts and post to their owners.
pub fn prepare(world: &mut World) {
    world.init_resource::<Mirrors>();
    world.init_resource::<Exchanged>();
    world.init_resource::<Outbox>();
    world.init_resource::<Inbox>();
    world.register_component::<Ghost>();
    world.register_component::<GlobalId>();
    world.register_component::<Position>();
    world.register_component::<Replicated>();
}

/// Adds `C` to what a world's ghosts carry of their owners.
pub fn add<C: Component + Clone>(world: &mut World) {
    world.get_resource_or_init::<Mirrors>().0.push(MirrorFns {
        changed: |world, entity, since| {
            let entity = world.entity(entity);
            let component = entity.get::<C>()?;
            let changed = since.is_none_or(|(last_run, this_run)| {
                entity
                    .get_change_ticks::<C>()
                    .is_some_and(|ticks: ComponentTicks| ticks.is_changed(last_run, this_run))
            });
            changed.then(|| Box::new(component.clone()) as Part)
        },
        apply: |world, entity, part, named| {
            let mut component = *part.downcast::<C>().expect("the part registered as C");
            C::map_entities(&mut component, named);
            world.entity_mut(entity).insert(component);
        },
    });
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
    let mut posted: Vec<Vec<Letter>> = (0..count).map(|_| Vec::new()).collect();
    for (index, world) in apps.iter_mut().enumerate() {
        let tick = world.change_tick();
        world.resource_mut::<Exchanged>().0 = Some(tick);
        for (owner, mut letter) in std::mem::take(&mut world.resource_mut::<Outbox>().0) {
            letter.from = Some(index);
            posted[owner].push(letter);
        }
    }
    for (world, letters) in apps.iter_mut().zip(posted) {
        world.resource_mut::<Inbox>().0.extend(letters);
    }
}

type Part = Box<dyn Any + Send + Sync>;

struct Letter {
    to: GlobalId,
    by: Option<GlobalId>,
    /// The world that posted, set as the letter goes out.
    from: Option<usize>,
    effect: Effect,
}

/// What this world posted to its neighbours this tick, by the world it goes to.
#[derive(Resource, Default)]
struct Outbox(Vec<(usize, Letter)>);

/// What neighbours posted to this world, landed by `deliver`.
#[derive(Resource, Default)]
struct Inbox(Vec<Letter>);

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
        .expect("prepare registers what the halo is scanned by");
    for (entity, &id, position) in owned.iter(world) {
        for neighbour in worlds.around(index, position.pos) {
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
