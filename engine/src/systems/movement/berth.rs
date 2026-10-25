use std::cmp::Ordering;

use bevy_ecs::prelude::*;

use super::spots::{SPACING, Spots, crowding, overlaps, spot_of};
use super::{MoveTarget, NavMesh, goto, mesh_in, moving, position, relocate};
use crate::core::math::{Offset, Pos};
use crate::core::units::{MeterPos, Meters};
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
        Some(match self {
            Aim::Point(point) => Sight {
                origin: point,
                footing: None,
                avoid: Vec::new(),
                nearest_first: false,
            },
            Aim::Actor(actor) => {
                let footing = position(world, actor)?;
                Sight {
                    origin: footing,
                    footing: Some(footing),
                    avoid: vec![footing],
                    nearest_first: moving(world, actor),
                }
            }
            Aim::Meeting(actor) => {
                let footing = position(world, actor)?;
                let origin = spot_of(world, actor).map_or(footing, |spot| spot.at);
                Sight {
                    origin,
                    footing: Some(footing),
                    avoid: vec![origin, footing],
                    nearest_first: false,
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
/// in reach, so an approach never wavers.
pub fn approach(world: &mut World, entity: Entity, aim: Aim, reach: Meters) -> Approach {
    let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
        return Approach::Unreachable;
    };
    if !moving(world, entity) && sight.reaches(at, reach) {
        let met = sight
            .footing
            .is_none_or(|footing| at.distance(footing) <= reach);
        return if met {
            Approach::Arrived
        } else {
            Approach::Approaching
        };
    }
    let kept = world
        .get::<MoveTarget>(entity)
        .is_some_and(|goal| sight.berths(goal.pos, reach));
    if kept {
        return Approach::Approaching;
    }
    match berth(world, entity, at, &sight, reach, sight.nearest_first) {
        Some(spot) => {
            goto(world, entity, spot);
            Approach::Approaching
        }
        None => Approach::Unreachable,
    }
}

/// Moves a resting actor in reach of what it aims at to a better berth: off the body of the actor it
/// aims at, or onto a spot overlapping at least two fewer, the smallest gap that evens a crowd out
/// without two actors trading places back and forth.
pub fn settle(world: &mut World, entity: Entity, aim: Aim, reach: Meters) {
    if moving(world, entity) || aim.actor() == Some(entity) {
        return;
    }
    let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
        return;
    };
    let crowd = crowding(world, entity, at) + 1;
    let on_target = aim
        .actor()
        .and_then(|actor| spot_of(world, actor))
        .is_some_and(|spot| overlaps(spot.at, at));
    if !on_target && crowd < 2 {
        return;
    }
    let Some(best) = berth(world, entity, at, &sight, reach, false) else {
        return;
    };
    if best != at && (on_target || crowding(world, entity, best) + 2 <= crowd) {
        goto(world, entity, best);
    }
}

/// `spot` when it is on the mesh and no body rests on it, else the nearest point on the mesh where a
/// body fits beside those already there.
pub fn place_beside(world: &World, area: AreaId, mesh: &NavMesh, spot: Pos<Meters>) -> Pos<Meters> {
    let spots = world.resource::<Spots>();
    mesh.nearest_where(spot, SPACING, |at| spots.crowding(area, at, None) == 0)
        .unwrap_or(spot)
}

/// Puts the actor down at `spot`, or beside it when someone else rests there.
pub fn relocate_beside(world: &mut World, entity: Entity, spot: Pos<Meters>) {
    world.entity_mut(entity).remove::<super::Spot>();
    let at = match (
        world.get::<AreaTag>(entity).map(|tag| tag.area),
        mesh_in(world, entity),
    ) {
        (Some(area), Some(mesh)) => place_beside(world, area, mesh, spot),
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
    let free = |point: Pos<Meters>| mesh.connects(at, point) && crowding(world, entity, point) == 0;
    if around(spot, reach).any(|point| mesh.walkable(point) && free(point)) {
        return spot;
    }
    mesh.nearest_where(spot, SPACING, free).unwrap_or(spot)
}

/// Whether the actor can walk from where it is to `point`.
pub fn reachable(world: &World, entity: Entity, point: Pos<Meters>) -> bool {
    let (Some(at), Some(mesh)) = (position(world, entity), mesh_in(world, entity)) else {
        return false;
    };
    mesh.connects(at, point)
}

struct Sight {
    origin: Pos<Meters>,
    footing: Option<Pos<Meters>>,
    avoid: Vec<Pos<Meters>>,
    nearest_first: bool,
}

impl Sight {
    fn reaches(&self, point: Pos<Meters>, reach: Meters) -> bool {
        point.distance(self.origin) <= reach
    }

    fn avoided(&self, point: Pos<Meters>) -> bool {
        self.avoid.iter().any(|&avoid| overlaps(avoid, point))
    }

    fn berths(&self, point: Pos<Meters>, reach: Meters) -> bool {
        self.reaches(point, reach) && !self.avoided(point)
    }
}

fn berth(
    world: &World,
    entity: Entity,
    at: Pos<Meters>,
    sight: &Sight,
    reach: Meters,
    nearest_first: bool,
) -> Option<Pos<Meters>> {
    let mesh = mesh_in(world, entity)?;
    let candidates: Vec<Candidate> = around(sight.origin, reach)
        .filter(|&point| mesh.walkable(point) && mesh.connects(at, point))
        .map(|point| Candidate {
            at: point,
            crowd: crowding(world, entity, point),
            distance: at.distance(point),
            avoided: sight.avoided(point),
        })
        .collect();
    candidates
        .iter()
        .filter(|candidate| !candidate.avoided)
        .min_by(|a, b| a.rank(b, nearest_first))
        .or_else(|| candidates.iter().min_by(|a, b| a.rank(b, nearest_first)))
        .map(|candidate| candidate.at)
}

struct Candidate {
    at: Pos<Meters>,
    crowd: u16,
    distance: Meters,
    avoided: bool,
}

impl Candidate {
    fn rank(&self, other: &Candidate, nearest_first: bool) -> Ordering {
        let emptiest = self.crowd.cmp(&other.crowd);
        let nearest = self.distance.0.total_cmp(&other.distance.0);
        let (first, second) = if nearest_first {
            (nearest, emptiest)
        } else {
            (emptiest, nearest)
        };
        first
            .then(second)
            .then(other.at.y.total_cmp(&self.at.y))
            .then(self.at.x.total_cmp(&other.at.x))
    }
}

/// The spots within `reach` of `origin` where bodies stand without overlapping: the origin, then
/// rings a body apart out to the reach.
fn around(origin: Pos<Meters>, reach: Meters) -> impl Iterator<Item = Pos<Meters>> {
    let rings = (reach.0 / SPACING.0).floor() as usize;
    let outer = (reach.0 - SPACING.0 * rings as f32 > RING_FUZZ).then_some(reach.0 - RING_FUZZ);
    let radii = (1..=rings)
        .map(move |ring| SPACING.0 * ring as f32)
        .chain(outer);
    std::iter::once(origin).chain(radii.flat_map(move |radius| {
        let count = ((std::f32::consts::TAU * radius / SPACING.0).floor() as usize).max(1);
        (0..count).map(move |index| {
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            origin + Offset::new(angle.cos() * radius, angle.sin() * radius)
        })
    }))
}

/// Keeps the outermost ring inside the reach despite rounding.
const RING_FUZZ: f32 = 1e-3;
