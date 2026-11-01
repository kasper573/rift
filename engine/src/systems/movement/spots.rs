use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;

use crate::core::chunk::Chunk;
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::ModelDef;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::class::ClassDef;
use crate::systems::player::PlayerDef;
use crate::systems::stat;

/// How much ground a walker's body takes: the walk mesh keeps its middle this far from what
/// blocks, and walkers keep this far from each other's bodies. Every body walks as the body of the
/// game's player does.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Bodies {
    pub radius: Meters,
}

impl Bodies {
    /// The bodies of `content`'s game: the hitbox of the model its player's class wears, when
    /// the settings, the player, its class and the model all loaded.
    pub fn of(content: &Content) -> Option<Bodies> {
        let player = content.try_built::<GameSettings>()?.player;
        let class = content.table::<PlayerDef>().get(player)?.class;
        let model = content.table::<ClassDef>().get(class)?.model;
        let hitbox = content.table::<ModelDef>().get(model)?.hitbox;
        Some(Bodies {
            radius: hitbox.radius,
        })
    }

    /// How far apart two bodies stand when they just touch.
    pub fn spacing(self) -> Meters {
        Meters(2.0 * self.radius.0)
    }

    /// Whether bodies resting at `a` and `b` would overlap; two just touching do not.
    pub fn overlap(self, a: Pos<Meters>, b: Pos<Meters>) -> bool {
        a.distance(b) < Meters(self.spacing().0 - TOUCH)
    }
}

/// The one place an actor means to stand: where its walk ends while it walks, else where it stands.
/// A spot never blocks a walk; it only steers where approaches, wanderers and placements choose to
/// stop, so bodies spread out rather than pile onto one point.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(on_insert = Spot::count, on_discard = Spot::uncount)]
pub struct Spot {
    pub area: AreaId,
    pub at: Pos<Meters>,
}

/// Every actor's [`Spot`] by area and chunk, kept in step with the spots as they come and go.
#[derive(Resource)]
pub struct Spots {
    bodies: Bodies,
    by_area: HashMap<AreaId, ChunkSpots>,
}

type ChunkSpots = HashMap<Chunk, Vec<(Entity, Pos<Meters>)>>;

impl Spots {
    pub fn new(bodies: Bodies) -> Spots {
        Spots {
            bodies,
            by_area: HashMap::default(),
        }
    }

    /// How many bodies resting on their spots in `area` a body at `at` would overlap, `except` for
    /// one actor's own.
    pub fn crowding(&self, area: AreaId, at: Pos<Meters>, except: Option<Entity>) -> u16 {
        self.by_area.get(&area).map_or(0, |spots| {
            Chunk::around(at, self.bodies.spacing())
                .filter_map(|chunk| spots.get(&chunk))
                .flatten()
                .filter(|(entity, spot)| Some(*entity) != except && self.bodies.overlap(*spot, at))
                .count() as u16
        })
    }
}

const TOUCH: f32 = 1e-3;

impl Spot {
    fn count(mut world: DeferredWorld, context: HookContext) {
        let Some(&spot) = world.get::<Spot>(context.entity) else {
            return;
        };
        world
            .resource_mut::<Spots>()
            .by_area
            .entry(spot.area)
            .or_default()
            .entry(Chunk::at(spot.at))
            .or_default()
            .push((context.entity, spot.at));
    }

    fn uncount(mut world: DeferredWorld, context: HookContext) {
        let Some(&spot) = world.get::<Spot>(context.entity) else {
            return;
        };
        let mut spots = world.resource_mut::<Spots>();
        let Some(area) = spots.by_area.get_mut(&spot.area) else {
            return;
        };
        let chunk = Chunk::at(spot.at);
        if let Some(filed) = area.get_mut(&chunk) {
            filed.retain(|(entity, _)| *entity != context.entity);
            if filed.is_empty() {
                area.remove(&chunk);
            }
        }
        if area.is_empty() {
            spots.by_area.remove(&spot.area);
        }
    }
}

pub fn spot_of(world: &World, entity: Entity) -> Option<Spot> {
    world.get::<Spot>(entity).copied()
}

/// How many actors other than `entity` have spots in its area a body at `at` would overlap.
pub fn crowding(world: &World, entity: Entity, at: Pos<Meters>) -> u16 {
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return 0;
    };
    world.resource::<Spots>().crowding(area, at, Some(entity))
}

/// Takes the spot at `pos` for the actor; a fallen actor takes none.
pub(super) fn claim(world: &mut World, entity: Entity, pos: Pos<Meters>) {
    if stat::is_dead(world, entity) {
        world.entity_mut(entity).remove::<Spot>();
        return;
    }
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return;
    };
    let spot = Spot { area, at: pos };
    if spot_of(world, entity) != Some(spot) {
        world.entity_mut(entity).insert(spot);
    }
}
