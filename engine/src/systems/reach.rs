use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::movement::{self, Aim, Approach, position};
use crate::systems::skill::SkillId;
use crate::systems::{stat, visibility};

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ReachIntent {
    pub target: ReachTarget,
    pub act: ReachAct,
}

/// The actor a character is casting at, replicated so its client knows whom a single-target skill
/// goes on without another click.
#[derive(Component, Serialize, Deserialize, MapEntities, Clone, Copy, Debug, PartialEq)]
pub struct Engaged(#[entities] pub Entity);

pub fn register(app: &mut bevy_app::App) {
    use crate::systems::mirror::AppMirrorExt;
    app.replicate::<Engaged>();
}

/// Keeps every character's `Engaged` in step with the actor its cast intent aims at.
pub fn mirror_engaged(
    world: &mut World,
    intents: &mut Intents,
    engaged: &mut QueryState<(Entity, &Engaged)>,
) {
    let mut wanted: std::collections::HashMap<Entity, Entity> = intents
        .iter(world)
        .filter(|(_, intent)| intent.cast().is_some())
        .filter_map(|(actor, intent)| Some((actor, intent.target.actor()?)))
        .collect();
    let stale: Vec<Entity> = engaged
        .iter(world)
        .filter(|(actor, engaged)| wanted.remove(actor) != Some(engaged.0))
        .map(|(actor, _)| actor)
        .collect();
    for actor in stale {
        world.entity_mut(actor).remove::<Engaged>();
    }
    let missing: Vec<(Entity, Entity)> = wanted.into_iter().collect();
    for (actor, target) in missing {
        if world.get::<Engaged>(actor) != Some(&Engaged(target)) {
            world.entity_mut(actor).insert(Engaged(target));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReachTarget {
    Actor(Entity),
    Point(Pos<Meters>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReachAct {
    /// `repeat` keeps the intent after a cast, so a basic attack lands again and again until the
    /// target falls or the actor turns away.
    Cast {
        skill: SkillId,
        repeat: bool,
    },
    Pickup,
    Interact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pursuit {
    Arrived,
    Approaching,
    Lost,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tether {
    pub anchor: Pos<Meters>,
    pub area: AreaId,
    pub range: Meters,
    pub with: Option<Entity>,
}

impl ReachTarget {
    pub fn actor(self) -> Option<Entity> {
        match self {
            ReachTarget::Actor(actor) => Some(actor),
            ReachTarget::Point(_) => None,
        }
    }

    pub fn point(self) -> Option<Pos<Meters>> {
        match self {
            ReachTarget::Point(point) => Some(point),
            ReachTarget::Actor(_) => None,
        }
    }

    pub fn position(self, world: &World) -> Option<Pos<Meters>> {
        match self {
            ReachTarget::Actor(actor) => position(world, actor),
            ReachTarget::Point(point) => Some(point),
        }
    }
}

impl ReachIntent {
    pub fn cast(&self) -> Option<SkillId> {
        match self.act {
            ReachAct::Cast { skill, .. } => Some(skill),
            ReachAct::Pickup | ReachAct::Interact => None,
        }
    }
}

impl Tether {
    pub fn around(
        world: &World,
        actor: Entity,
        range: Meters,
        with: Option<Entity>,
    ) -> Option<Tether> {
        Some(Tether {
            anchor: position(world, actor)?,
            area: world.get::<AreaTag>(actor)?.area,
            range,
            with,
        })
    }

    pub fn holds(&self, world: &World, actor: Entity) -> bool {
        !stat::is_dead(world, actor)
            && world.get::<AreaTag>(actor).map(|tag| tag.area) == Some(self.area)
            && position(world, actor).is_some_and(|at| at.distance(self.anchor) <= self.range)
            && self.with.is_none_or(|with| {
                stat::stands(world, with) && visibility::coexist(world, actor, with)
            })
    }
}

pub fn intend(world: &mut World, actor: Entity, target: ReachTarget, act: ReachAct) {
    movement::halt(world, actor);
    world.entity_mut(actor).insert(ReachIntent { target, act });
}

pub type Intents = QueryState<(Entity, &'static ReachIntent)>;

/// Every actor reaching for another actor with exactly this act.
pub fn intending(world: &World, intents: &mut Intents, act: ReachAct) -> Vec<(Entity, Entity)> {
    intents
        .iter(world)
        .filter(|(_, intent)| intent.act == act)
        .filter_map(|(actor, intent)| Some((actor, intent.target.actor()?)))
        .collect()
}

pub fn casting(world: &World, intents: &mut Intents) -> Vec<(Entity, ReachIntent)> {
    intents
        .iter(world)
        .filter(|(_, intent)| intent.cast().is_some())
        .map(|(actor, intent)| (actor, *intent))
        .collect()
}

pub fn current(world: &World, actor: Entity) -> Option<ReachIntent> {
    world.get::<ReachIntent>(actor).copied()
}

/// The actor a cast is aimed at, while the actor is casting at one.
pub fn attacking(world: &World, actor: Entity) -> Option<Entity> {
    current(world, actor)
        .filter(|intent| intent.cast().is_some())
        .and_then(|intent| intent.target.actor())
}

pub fn intent(world: &World, actor: Entity, act: ReachAct) -> Option<Entity> {
    current(world, actor)
        .filter(|intent| intent.act == act)
        .and_then(|intent| intent.target.actor())
}

/// Walks the actor into reach of what it intends to act on, by way of [`movement::approach`], and
/// tells whether it's there. Two actors coming for each other meet between them, each covering the
/// share of the gap its pace allows, rather than both heading for where the other stands.
pub fn pursue(world: &mut World, actor: Entity, range: Meters) -> Pursuit {
    let Some(target) = pursued(world, actor) else {
        return Pursuit::Lost;
    };
    let reach = range;
    if let ReachTarget::Actor(target) = target
        && let Some(meeting) = meeting_point(world, actor, target, reach)
    {
        movement::goto(world, actor, meeting);
        return Pursuit::Approaching;
    }
    match movement::approach(world, actor, aim(world, actor, target), reach) {
        Approach::Arrived => Pursuit::Arrived,
        Approach::Approaching => Pursuit::Approaching,
        Approach::Unreachable => Pursuit::Lost,
    }
}

/// Spreads an actor that has arrived out over the ground in reach, by way of
/// [`movement::settle`], while it waits to act again.
pub fn settle(world: &mut World, actor: Entity, range: Meters) {
    if let Some(target) = pursued(world, actor) {
        let aim = aim(world, actor, target);
        movement::settle(world, actor, aim, range);
    }
}

/// Whether an actor can still be reached for: alive, in the same area and able to meet.
pub fn pursuable(world: &World, actor: Entity, target: Entity) -> bool {
    stat::stands(world, target)
        && area::same_area(world, actor, target)
        && visibility::coexist(world, actor, target)
}

pub fn forget(world: &mut World, actor: Entity) {
    drop_intent(world, actor);
    movement::halt(world, actor);
}

/// Gives up reaching for anything, leaving the actor's walk as it is.
pub fn drop_intent(world: &mut World, actor: Entity) {
    world.entity_mut(actor).remove::<ReachIntent>();
}

fn pursued(world: &World, actor: Entity) -> Option<ReachTarget> {
    let target = current(world, actor)?.target;
    if stat::is_dead(world, actor) {
        return None;
    }
    match target {
        ReachTarget::Actor(target) => {
            pursuable(world, actor, target).then_some(ReachTarget::Actor(target))
        }
        ReachTarget::Point(_) => Some(target),
    }
}

fn aim(world: &World, actor: Entity, target: ReachTarget) -> Aim {
    match target {
        ReachTarget::Point(point) => Aim::Point(point),
        ReachTarget::Actor(target) if coming_for(world, target, actor) => Aim::Meeting(target),
        ReachTarget::Actor(target) => Aim::Actor(target),
    }
}

fn coming_for(world: &World, actor: Entity, target: Entity) -> bool {
    current(world, actor).is_some_and(|intent| intent.target.actor() == Some(target))
}

/// Where an actor out of reach of one coming for it heads while neither has set out yet: the point
/// on the line between them that each reaches at once, going at their own paces.
fn meeting_point(
    world: &World,
    actor: Entity,
    target: Entity,
    reach: Meters,
) -> Option<Pos<Meters>> {
    if !coming_for(world, target, actor)
        || movement::moving(world, actor)
        || movement::moving(world, target)
    {
        return None;
    }
    let at = position(world, actor)?;
    let there = position(world, target)?;
    if at.distance(there) <= reach {
        return None;
    }
    let pace = movement::speed_of(world, actor).0.0;
    let theirs = movement::speed_of(world, target).0.0;
    if pace + theirs <= 0.0 {
        return None;
    }
    let meeting = at.lerp(there, pace / (pace + theirs));
    (meeting.distance(there) > movement::SPACING
        && meeting.distance(at) > movement::SPACING
        && movement::reachable(world, actor, meeting))
    .then_some(meeting)
}
