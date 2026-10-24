use std::cmp::{Ordering, Reverse};

use bevy_ecs::prelude::*;

use super::claims::{TileClaim, TileClaims, claim_of, others_on};
use super::{Grid, MoveTarget, Navigator, Path, airborne, goto, moving, position, relocate};
use crate::core::math::Pos;
use crate::core::units::{Cell, CellPos, Meters, TilePos};
use crate::systems::area::{AreaId, AreaTag};

/// What an approach ends beside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    Point(Pos<Meters>),
    /// An actor or thing where it stands, or mid-step, where its step lands.
    Actor(Entity),
    /// An actor coming for the approacher, met at the tile it claimed.
    Meeting(Entity),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approach {
    /// Resting on a tile in reach.
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
                avoid: [None, None],
                nearest_first: false,
            },
            Aim::Actor(actor) => {
                let footing = footing(world, actor)?;
                Sight {
                    origin: footing,
                    footing: Some(footing),
                    avoid: [Some(footing.cell()), None],
                    nearest_first: moving(world, actor),
                }
            }
            Aim::Meeting(actor) => {
                let footing = footing(world, actor)?;
                let origin = claim_of(world, actor).map_or(footing, |claim| claim.cell.center());
                Sight {
                    origin,
                    footing: Some(footing),
                    avoid: [Some(origin.cell()), Some(footing.cell())],
                    nearest_first: false,
                }
            }
        })
    }
}

/// Walks the actor to a berth in `reach` of what it aims at, and tells when it rests there.
///
/// A berth is the tile in reach claimed by the fewest others, the nearest of those, then the
/// southernmost and westernmost, never the tile an actor aimed at stands on while another is in
/// reach; while that actor walks, nearness counts before emptiness. A berth is kept while it stays
/// in reach, so an approach never wavers, and one that lies in reach mid-walk is still walked to,
/// so no one stops mid-step onto a tile nobody chose.
pub fn approach(world: &mut World, entity: Entity, aim: Aim, reach: Meters) -> Approach {
    let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
        return Approach::Unreachable;
    };
    if !moving(world, entity) && at.on_center() && sight.reaches(at.cell(), reach) {
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
        .is_some_and(|goal| sight.berths(goal.pos.cell(), reach));
    if kept {
        return Approach::Approaching;
    }
    match berth(world, entity, at, &sight, reach, sight.nearest_first) {
        Some(cell) => {
            goto(world, entity, cell.center());
            Approach::Approaching
        }
        None => Approach::Unreachable,
    }
}

/// Moves a resting actor in reach of what it aims at to a better berth: off the tile the actor it
/// aims at claimed, or onto one claimed by at least two fewer, the smallest gap that evens a crowd
/// out without two actors trading places back and forth.
pub fn settle(world: &mut World, entity: Entity, aim: Aim, reach: Meters) {
    if moving(world, entity) || aim.actor() == Some(entity) {
        return;
    }
    let (Some(at), Some(sight)) = (position(world, entity), aim.sight(world)) else {
        return;
    };
    let here = at.cell();
    let crowd = others_on(world, entity, here) + 1;
    let on_target = aim
        .actor()
        .and_then(|actor| claim_of(world, actor))
        .is_some_and(|claim| claim.cell == here);
    if !on_target && crowd < 2 {
        return;
    }
    let Some(best) = berth(world, entity, at, &sight, reach, false) else {
        return;
    };
    if best != here && (on_target || others_on(world, entity, best) + 2 <= crowd) {
        goto(world, entity, best.center());
    }
}

/// The walkable tile at `spot` when no one claims it, else the nearest one no one claims:
/// southernmost, then westernmost, of equally near ones.
pub fn place_beside(world: &World, area: AreaId, grid: &Grid, spot: Pos<Meters>) -> Pos<Meters> {
    let claims = world.resource::<TileClaims>();
    grid.nearest_walkable_where(spot, |cell| claims.on(area, cell) == 0)
        .unwrap_or(spot)
}

/// Puts the actor down at `spot`, or beside it when someone else claims it.
pub fn relocate_beside(world: &mut World, entity: Entity, spot: Pos<Meters>) {
    world.entity_mut(entity).remove::<TileClaim>();
    let at = match (
        world.get::<AreaTag>(entity).map(|tag| tag.area),
        Navigator::of(world, entity),
    ) {
        (Some(area), Some(navigator)) => place_beside(world, area, navigator.grid, spot),
        _ => spot,
    };
    relocate(world, entity, at);
}

/// `spot` while some tile within `reach` of it is free for the actor, else the nearest free tile
/// it can walk to, so an actor sent to a taken spot stops beside it rather than on someone.
pub fn vacant_near(world: &World, entity: Entity, spot: Pos<Meters>, reach: Meters) -> Pos<Meters> {
    let (Some(at), Some(navigator)) = (position(world, entity), Navigator::of(world, entity))
    else {
        return spot;
    };
    let free = |cell: CellPos| {
        navigator.grid.connects(at.cell(), cell) && others_on(world, entity, cell) == 0
    };
    if around(spot, reach).any(|cell| cell.center().distance(spot) <= reach && free(cell)) {
        return spot;
    }
    navigator
        .grid
        .nearest_walkable_where(spot, free)
        .unwrap_or(spot)
}

/// Whether the actor can walk from where it is to `cell`.
pub fn reachable(world: &World, entity: Entity, cell: CellPos) -> bool {
    let (Some(at), Some(navigator)) = (position(world, entity), Navigator::of(world, entity))
    else {
        return false;
    };
    navigator.grid.connects(at.cell(), cell)
}

/// Where an actor stands, or mid-step on foot, the tile its step lands on. A flyer goes straight
/// for where it's headed, however far, so it is met where it is.
pub fn footing(world: &World, entity: Entity) -> Option<Pos<Meters>> {
    let at = position(world, entity)?;
    let landing = world
        .get::<Path>(entity)
        .and_then(Path::next)
        .filter(|_| !at.on_center() && !airborne(world, entity));
    Some(landing.map_or(at, Cell::center))
}

struct Sight {
    origin: Pos<Meters>,
    footing: Option<Pos<Meters>>,
    avoid: [Option<CellPos>; 2],
    nearest_first: bool,
}

impl Sight {
    fn reaches(&self, cell: CellPos, reach: Meters) -> bool {
        cell.center().distance(self.origin) <= reach
    }

    fn berths(&self, cell: CellPos, reach: Meters) -> bool {
        self.reaches(cell, reach) && !self.avoid.contains(&Some(cell))
    }
}

fn berth(
    world: &World,
    entity: Entity,
    at: Pos<Meters>,
    sight: &Sight,
    reach: Meters,
    nearest_first: bool,
) -> Option<CellPos> {
    let navigator = Navigator::of(world, entity)?;
    let start = at.cell();
    let candidates: Vec<Candidate> = around(sight.origin, reach)
        .filter(|&cell| sight.reaches(cell, reach) && navigator.grid.connects(start, cell))
        .map(|cell| Candidate {
            cell,
            claims: others_on(world, entity, cell),
            distance: at.distance(cell.center()),
            avoided: sight.avoid.contains(&Some(cell)),
        })
        .collect();
    candidates
        .iter()
        .filter(|candidate| !candidate.avoided)
        .min_by(|a, b| a.rank(b, nearest_first))
        .or_else(|| candidates.iter().min_by(|a, b| a.rank(b, nearest_first)))
        .map(|candidate| candidate.cell)
}

struct Candidate {
    cell: CellPos,
    claims: u16,
    distance: Meters,
    avoided: bool,
}

impl Candidate {
    fn rank(&self, other: &Candidate, nearest_first: bool) -> Ordering {
        let emptiest = self.claims.cmp(&other.claims);
        let nearest = self.distance.0.total_cmp(&other.distance.0);
        let (first, second) = if nearest_first {
            (nearest, emptiest)
        } else {
            (emptiest, nearest)
        };
        first
            .then(second)
            .then(Reverse(self.cell.y).cmp(&Reverse(other.cell.y)))
            .then(self.cell.x.cmp(&other.cell.x))
    }
}

fn around(origin: Pos<Meters>, reach: Meters) -> impl Iterator<Item = CellPos> {
    let center = origin.cell();
    let reach = reach.0.ceil() as i32;
    (-reach..=reach).flat_map(move |dy| (-reach..=reach).map(move |dx| center.step((dx, dy))))
}
