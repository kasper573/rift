use bevy_ecs::prelude::*;
use rayon::prelude::*;
use strum::{IntoStaticStr, VariantArray};

use super::{StatKind, Stats, Vital, base, effective};
use crate::core::content::{Fixture, Loading};
use crate::core::par;
use crate::core::tick::{Cached, Planner};
use crate::core::vfx::{SiteKind, VfxDef, VfxSite, check_fixtures_played};
use crate::systems::actor::Hurt;
use crate::systems::area::vfx::play_fixture;
use crate::systems::notification;
use crate::systems::notification::pop::PopShader;
use crate::systems::status::boost::Boosts;

/// The vfx every game provides, played on an actor where a heal lands. Where damage lands, the
/// blow's own vfx shows it and the actor flinches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum VitalVfx {
    Healed,
}

impl Fixture for VitalVfx {
    type Row = VfxDef;
    const FOLDER: Option<&'static str> = Some("vfx");
}

pub fn check_vfx(loading: &mut Loading) {
    check_fixtures_played::<VitalVfx>(loading, |_| &[SiteKind::Actor]);
}

/// Worn down by damage like anyone, but always left standing: a blow that would fell it leaves its
/// health as it was.
#[derive(Component, Clone, Copy, Debug)]
pub struct Immortal;

pub fn current_health(world: &World, entity: Entity) -> f32 {
    current(world, entity, Vital::Health)
}

pub fn max_health(world: &World, entity: Entity) -> f32 {
    most(world, entity, Vital::Health)
}

/// How much of `vital` the entity holds.
pub fn current(world: &World, entity: Entity, vital: Vital) -> f32 {
    base(world, entity, vital.held())
}

/// How much of `vital` the entity holds at most.
pub fn most(world: &World, entity: Entity, vital: Vital) -> f32 {
    effective(world, entity, vital.most())
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Stats>(entity).is_some_and(dead)
}

/// Whether an entity is there and not fallen; what has no health never falls.
pub fn stands(world: &World, entity: Entity) -> bool {
    world.get_entity(entity).is_ok() && !is_dead(world, entity)
}

pub fn dead(stats: &Stats) -> bool {
    stats.get(StatKind::Health) <= 0.0
}

pub fn apply_damage(world: &mut World, entity: Entity, amount: f32) {
    if is_dead(world, entity) {
        return;
    }
    let immortal = world.get::<Immortal>(entity).is_some();
    set(world, entity, Vital::Health, |health, _| {
        match health - amount {
            left if left <= 0.0 && immortal => health,
            left => left.max(0.0),
        }
    });
    if amount > 0.0 {
        let blows = world.get::<Hurt>(entity).map_or(0, |hurt| hurt.0);
        world.entity_mut(entity).insert(Hurt(blows + 1));
        let shown = whole(amount).to_string();
        notification::pop_as(world, entity, shown, PopShader::DamagePop);
    }
}

/// A heal that lands as an event, shown where it lands; the slow trickle of regeneration is
/// `restore`.
pub fn heal(world: &mut World, entity: Entity, amount: f32) {
    if is_dead(world, entity) || amount <= 0.0 {
        return;
    }
    let before = current_health(world, entity);
    restore(world, entity, Vital::Health, amount);
    if current_health(world, entity) > before {
        play_fixture(world, VitalVfx::Healed, VfxSite::Actor(entity));
        let shown = format!("+{}", whole(amount));
        notification::pop_as(world, entity, shown, PopShader::HealPop);
    }
}

/// Gives the entity `amount` of `vital` back, up to its maximum. The fallen regain nothing.
pub fn restore(world: &mut World, entity: Entity, vital: Vital, amount: f32) {
    if !is_dead(world, entity) {
        set(world, entity, vital, |held, most| (held + amount).min(most));
    }
}

pub fn refill(world: &mut World, entity: Entity, vital: Vital) {
    if !is_dead(world, entity) {
        set(world, entity, vital, |_, most| most);
    }
}

/// Takes `amount` of `vital` from the entity, down to none of it. Health taken this way is not
/// a blow: nobody is to blame for it, and it shows nowhere but in what is left.
pub fn drain(world: &mut World, entity: Entity, vital: Vital, amount: f32) {
    if !is_dead(world, entity) {
        set(world, entity, vital, |held, _| (held - amount).max(0.0));
    }
}

pub fn revive(world: &mut World, entity: Entity) {
    for vital in Vital::BOTH {
        set(world, entity, vital, |_, most| most);
    }
}

/// Holds every actor's health and mana within their maximums: found on every core among the
/// actors whose share of either could have risen past it, written in the order the actors were
/// found. A vital passes its maximum when it changes or when a boost comes or goes; what else
/// feeds the maximum only ever raises it.
#[derive(Default)]
pub struct SettleVitals {
    living: Cached<Entity, MaximumMoved>,
    settled: Vec<(Entity, Vital, f32)>,
}

/// Actors whose health or mana, or the maximum of either, may have changed.
type MaximumMoved = Or<(Changed<Stats>, Changed<Boosts>)>;

impl Planner for SettleVitals {
    fn prepare(&mut self, world: &mut World) {
        self.living.ready(world);
    }

    fn plan(&mut self, world: &World) {
        let entities: Vec<Entity> = self.living.query(world).iter().collect();
        self.settled = entities
            .par_iter()
            .with_min_len(par::grain(entities.len()))
            .flat_map_iter(|&entity| {
                Vital::BOTH.into_iter().filter_map(move |vital| {
                    let &held = world.get::<Stats>(entity)?.0.get(&vital.held())?;
                    let settled = held.min(most(world, entity, vital));
                    (settled != held).then_some((entity, vital, settled))
                })
            })
            .collect();
    }

    fn apply(&mut self, world: &mut World) {
        for (entity, vital, settled) in self.settled.drain(..) {
            if let Some(mut stats) = world.get_mut::<Stats>(entity) {
                stats.0.insert(vital.held(), settled);
            }
        }
    }
}

/// Sets how much of `vital` the entity holds, from what it holds and what it holds at most. An
/// entity that has none of the vital to its name keeps none.
fn set(world: &mut World, entity: Entity, vital: Vital, f: impl Fn(f32, f32) -> f32) {
    let most = most(world, entity, vital);
    let Some(&held) = world
        .get::<Stats>(entity)
        .and_then(|stats| stats.0.get(&vital.held()))
    else {
        return;
    };
    let settled = f(held.min(most), most);
    if settled != held
        && let Some(mut stats) = world.get_mut::<Stats>(entity)
    {
        stats.0.insert(vital.held(), settled);
    }
}

/// An amount as players read it: whole, and never a zero for something that landed.
fn whole(amount: f32) -> u32 {
    amount.round().max(1.0) as u32
}
