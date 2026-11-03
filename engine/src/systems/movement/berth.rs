use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use super::spots::{Bodies, Spots, crowding, spot_of};
use super::{MoveTarget, NavMesh, goto, mesh_in, moving, position, relocate, speed_of};
use crate::core::math::{Offset, Pos};
use crate::core::units::{MeterPos, Meters, MetersPerSec};
use crate::systems::area::{AreaId, AreaTag};

/// What an approach ends beside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Pos<Meters>),
    /// An actor or thing where it stands.
    Actor(Entity),
    /// An actor coming for the approacher, met at the spot it took.
    Meeting(Entity),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    /// Resting in reach.
    Arrived,
    Approaching,
    Unreachable,
}

impl Aim {
    pub fn actor(self) -> Option<Entity> {
        match self {
            Aim::Point(_) => None,
            Aim::Actor(actor) | Aim::Meeting(actor) => Some(actor),
        }
    }

    fn sight(self, world: &World) -> Option<Sight> {
        let bodies = *world.resource::<Bodies>();
        Some(match self {
            Aim::Point(point) => Sight {
                origin: point,
                actor: None,
                footing: None,
                avoid: Vec::new(),
                walking: false,
                bodies,
            },
            Aim::Actor(actor) => {
                let footing = position(world, actor)?;
                Sight {
                    origin: footing,
                    actor: Some(actor),
                    footing: Some(footing),
                    avoid: vec![footing],
                    walking: moving(world, actor),
                    bodies,
                }
            }
            Aim::Meeting(actor) => {
                let footing = position(world, actor)?;
                let origin = spot_of(world, actor).map_or(footing, |spot| spot.at);
                Sight {
                    origin,
                    actor: Some(actor),
                    footing: Some(footing),
                    avoid: vec![origin, footing],
                    walking: false,
                    bodies,
                }
            }
        })
    }
}

/// Walks the actor to a berth in `reach` of what it aims at, and tells when it rests there.
///
/// A berth is the spot in reach whose body would overlap the fewest others, the nearest of those,
/// then the southernmost and westernmost, never one overlapping the actor aimed at while another is
/// in reach; while that actor walks, nearness counts before emptiness. A berth is kept while it stays
/// in reach, so an approach never wavers. An actor walking away is closed on to a body's width inside
/// the reach, and met wherever the approacher gets that near, so it is still in reach once the
/// approacher has stopped. An actor that can't take a step, held fast or never moving at all, waits
/// where it stands for what it aims at to come in reach.
pub fn approach(world: &mut World, entity: Entity, aim: Aim, reach: Meters) -> Approach {
    ApproachPlan::of(world, entity, aim, reach).apply(world, entity)
}

/// What an [`approach`] comes to, worked out from the world as it stood, so many can be planned on
/// every core and written in order: the berths it may choose from are found on the ground, which
/// doesn't change meanwhile, and the one chosen among them by the crowd as it stands when the plan
/// is applied.
pub enum ApproachPlan {
    Unreachable,
    /// Standing in reach of what it aims at: `met` once that is in reach of the actor's body too.
    Resting {
        halt: bool,
        met: bool,
    },
    /// Waiting where it stands or walking on to the berth it has.
    Holding {
        halt: bool,
    },
    /// Choosing a berth to walk to.
    Berthing {
        halt: bool,
        berths: Berths,
        nearest_first: bool,
    },
}

impl ApproachPlan {
    pub fn of(world: &World, entity: Entity, aim: Aim, reach: Meters) -> ApproachPlan {
        let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
            return ApproachPlan::Unreachable;
        };
        let rooted = speed_of(world, entity) <= MetersPerSec(Meters(0.0));
        let caught = sight.walking && sight.reaches(at, sight.closing(reach));
        let halt = moving(world, entity) && (rooted || caught);
        if (halt || !moving(world, entity)) && sight.reaches(at, reach) {
            let met = sight
                .footing
                .is_none_or(|footing| at.distance(footing) <= reach);
            return ApproachPlan::Resting { halt, met };
        }
        if rooted {
            return ApproachPlan::Holding { halt };
        }
        let reach = if sight.walking {
            sight.closing(reach)
        } else {
            reach
        };
        let kept = !halt
            && world
                .get::<MoveTarget>(entity)
                .is_some_and(|goal| sight.berths(goal.pos, reach));
        if kept {
            return ApproachPlan::Holding { halt };
        }
        ApproachPlan::Berthing {
            halt,
            berths: Berths::within(world, entity, at, &sight, reach),
            nearest_first: sight.walking,
        }
    }

    pub fn arrived(&self) -> bool {
        matches!(self, ApproachPlan::Resting { met: true, .. })
    }

    pub fn apply(self, world: &mut World, entity: Entity) -> Approach {
        match self {
            ApproachPlan::Unreachable => Approach::Unreachable,
            ApproachPlan::Resting { halt, met } => {
                halt_if(world, entity, halt);
                if met {
                    Approach::Arrived
                } else {
                    Approach::Approaching
                }
            }
            ApproachPlan::Holding { halt } => {
                halt_if(world, entity, halt);
                Approach::Approaching
            }
            ApproachPlan::Berthing {
                halt,
                berths,
                nearest_first,
            } => {
                halt_if(world, entity, halt);
                match berths.pick(world, entity, nearest_first) {
                    Some(spot) => {
                        goto(world, entity, spot);
                        Approach::Approaching
                    }
                    None => Approach::Unreachable,
                }
            }
        }
    }
}

/// Moves a resting actor in reach of what it aims at to a better berth: off the body of the actor it
/// aims at, or onto a spot overlapping at least two fewer, the smallest gap that evens a crowd out
/// without two actors trading places back and forth.
pub fn settle(world: &mut World, entity: Entity, aim: Aim, reach: Meters) {
    if let Some(berths) = Berths::for_settling(world, entity, aim, reach) {
        berths.settle(world, entity);
    }
}

/// The spots in reach of what an actor aims at that it can walk to, found on the ground as it
/// stands, ranked among themselves by nearness alone; the crowd on each is read as one is picked.
pub struct Berths {
    aimed_at: Option<Entity>,
    candidates: Vec<Candidate>,
    bodies: Bodies,
}

impl Berths {
    /// The berths an actor resting at `at` may [`settle`] on, when its berth is worth leaving:
    /// overlapping the body of the actor it aims at, or crowded by another.
    pub fn for_settling(world: &World, entity: Entity, aim: Aim, reach: Meters) -> Option<Berths> {
        if moving(world, entity) || aim.actor() == Some(entity) {
            return None;
        }
        let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
            return None;
        };
        let berths = Berths::within(world, entity, at, &sight, reach);
        berths.worth_leaving(world, entity, at).then_some(berths)
    }

    /// Moves the actor to the best of these berths when that evens the crowd out, as it stands now.
    pub fn settle(&self, world: &mut World, entity: Entity) {
        let Some(at) = position(world, entity) else {
            return;
        };
        if !self.worth_leaving(world, entity, at) {
            return;
        }
        let Some(best) = self.pick(world, entity, false) else {
            return;
        };
        let crowd = crowding(world, entity, at) + 1;
        if best != at && (self.on_target(world, at) || crowding(world, entity, best) + 2 <= crowd) {
            goto(world, entity, best);
        }
    }

    fn within(
        world: &World,
        entity: Entity,
        at: Pos<Meters>,
        sight: &Sight,
        reach: Meters,
    ) -> Berths {
        let aimed_at = sight.actor;
        let Some((mesh, from)) = mesh_in(world, entity).and_then(|mesh| {
            let island = mesh.island(at)?;
            Some((mesh, island))
        }) else {
            return Berths {
                aimed_at,
                candidates: Vec::new(),
                bodies: sight.bodies,
            };
        };
        let mut candidates: Vec<Candidate> = around(sight.origin, reach, sight.bodies)
            .filter(|&point| mesh.island(point) == Some(from))
            .map(|point| Candidate {
                at: point,
                distance: at.distance(point),
                avoided: sight.avoided(point),
            })
            .collect();
        candidates.sort_by(Candidate::nearest);
        Berths {
            aimed_at,
            candidates,
            bodies: sight.bodies,
        }
    }

    /// The berth whose body would overlap the fewest others as the crowd stands now, the nearest of
    /// those (the nearest outright, when `nearest_first`), then the southernmost and westernmost; one
    /// to avoid only when no other is in reach.
    fn pick(&self, world: &World, entity: Entity, nearest_first: bool) -> Option<Pos<Meters>> {
        let crowd = |candidate: &Candidate| crowding(world, entity, candidate.at);
        self.best(|candidate| !candidate.avoided, crowd, nearest_first)
            .or_else(|| self.best(|_| true, crowd, nearest_first))
    }

    fn best(
        &self,
        among: impl Fn(&Candidate) -> bool,
        crowd: impl Fn(&Candidate) -> u16,
        nearest_first: bool,
    ) -> Option<Pos<Meters>> {
        let mut best: Option<(u16, &Candidate)> = None;
        for candidate in self.candidates.iter().filter(|candidate| among(candidate)) {
            if let Some((best_crowd, best_candidate)) = best
                && (best_crowd == 0
                    || (nearest_first && candidate.distance != best_candidate.distance))
            {
                break;
            }
            let crowd = crowd(candidate);
            if best.is_none_or(|(best_crowd, _)| crowd < best_crowd) {
                best = Some((crowd, candidate));
            }
        }
        best.map(|(_, candidate)| candidate.at)
    }

    fn worth_leaving(&self, world: &World, entity: Entity, at: Pos<Meters>) -> bool {
        self.on_target(world, at) || crowding(world, entity, at) >= 1
    }

    fn on_target(&self, world: &World, at: Pos<Meters>) -> bool {
        self.aimed_at
            .and_then(|actor| spot_of(world, actor))
            .is_some_and(|spot| self.bodies.overlap(spot.at, at))
    }
}

/// `spot` when it is on the mesh and no body rests on it, else the nearest point on the mesh where a
/// body fits beside those already there.
pub fn place_beside(world: &World, area: AreaId, mesh: &NavMesh, spot: Pos<Meters>) -> Pos<Meters> {
    let spots = world.resource::<Spots>();
    let spacing = world.resource::<Bodies>().spacing();
    mesh.nearest_where(spot, spacing, |at| spots.crowding(area, at, None) == 0)
        .unwrap_or(spot)
}

/// Puts the actor down at `spot`, or beside it when someone else rests there.
pub fn relocate_beside(world: &mut World, entity: Entity, spot: Pos<Meters>) {
    world.entity_mut(entity).remove::<super::Spot>();
    let at = match (
        world.get::<AreaTag>(entity).map(|tag| tag.area),
        mesh_in(world, entity),
    ) {
        (Some(area), Some(mesh)) => place_beside(world, area, &mesh, spot),
        _ => spot,
    };
    relocate(world, entity, at);
}

/// `spot` while some point within `reach` of it is free for the actor, else the nearest free point
/// it can walk to, so an actor sent to a taken spot stops beside it rather than on someone.
pub fn vacant_near(world: &World, entity: Entity, spot: Pos<Meters>, reach: Meters) -> Pos<Meters> {
    let (Some(at), Some(mesh)) = (position(world, entity), mesh_in(world, entity)) else {
        return spot;
    };
    let Some(from) = mesh.island(at) else {
        return spot;
    };
    let free = |point: Pos<Meters>| {
        mesh.island(point) == Some(from) && crowding(world, entity, point) == 0
    };
    let bodies = *world.resource::<Bodies>();
    if around(spot, reach, bodies).any(free) {
        return spot;
    }
    mesh.nearest_where(spot, bodies.spacing(), free)
        .unwrap_or(spot)
}

/// Whether the actor can walk from where it is to `point`.
pub fn reachable(world: &World, entity: Entity, point: Pos<Meters>) -> bool {
    let (Some(at), Some(mesh)) = (position(world, entity), mesh_in(world, entity)) else {
        return false;
    };
    mesh.connects(at, point)
}

fn halt_if(world: &mut World, entity: Entity, halt: bool) {
    if halt {
        super::halt(world, entity);
    }
}

struct Sight {
    origin: Pos<Meters>,
    /// The actor aimed at, where it stands.
    actor: Option<Entity>,
    footing: Option<Pos<Meters>>,
    avoid: Vec<Pos<Meters>>,
    walking: bool,
    bodies: Bodies,
}

impl Sight {
    /// How near an approacher closes on an actor walking away: a body's width inside its reach,
    /// but never onto the actor's body, so stopping takes the approacher no further than the reach.
    fn closing(&self, reach: Meters) -> Meters {
        let spacing = self.bodies.spacing();
        Meters((reach.0 - spacing.0).max(spacing.0).min(reach.0))
    }

    fn reaches(&self, point: Pos<Meters>, reach: Meters) -> bool {
        point.distance(self.origin) <= reach
    }

    fn avoided(&self, point: Pos<Meters>) -> bool {
        self.avoid
            .iter()
            .any(|&avoid| self.bodies.overlap(avoid, point))
    }

    fn berths(&self, point: Pos<Meters>, reach: Meters) -> bool {
        self.reaches(point, reach) && !self.avoided(point)
    }
}

struct Candidate {
    at: Pos<Meters>,
    distance: Meters,
    avoided: bool,
}

impl Candidate {
    fn nearest(&self, other: &Candidate) -> Ordering {
        self.distance
            .0
            .total_cmp(&other.distance.0)
            .then(other.at.y.total_cmp(&self.at.y))
            .then(self.at.x.total_cmp(&other.at.x))
    }
}

/// The spots within `reach` of `origin` where bodies stand without overlapping: the origin, then
/// rings a body apart out to the reach.
fn around(origin: Pos<Meters>, reach: Meters, bodies: Bodies) -> impl Iterator<Item = Pos<Meters>> {
    let spacing = bodies.spacing().0;
    let rings = (reach.0 / spacing).floor() as usize;
    let outer = (reach.0 - spacing * rings as f32 > RING_FUZZ).then_some(reach.0 - RING_FUZZ);
    let radii = (1..=rings)
        .map(move |ring| spacing * ring as f32)
        .chain(outer);
    std::iter::once(origin).chain(radii.flat_map(move |radius| {
        let count = ((std::f32::consts::TAU * radius / spacing).floor() as usize).max(1);
        (0..count).map(move |index| {
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            origin + Offset::new(angle.cos() * radius, angle.sin() * radius)
        })
    }))
}

/// Keeps the outermost ring inside the reach despite rounding.
const RING_FUZZ: f32 = 1e-3;
