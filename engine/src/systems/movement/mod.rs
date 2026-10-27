mod berth;
mod navmesh;
mod predict;
mod spots;

pub use berth::{
    Aim, Approach, approach, place_beside, reachable, relocate_beside, settle, vacant_near,
};
pub use navmesh::{AGENT_RADIUS, Island, NavMesh};
pub use predict::request_walk;
pub use spots::{SPACING, Spot, Spots, crowding, spot_of};

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::geometry::Radians;
use crate::core::interpolate::{Interpolate, InterpolatePlugin, Jumps, RenderValueSystems};
use crate::core::math::{Offset, Pos};
use crate::core::time::Seconds;
use crate::core::units::{MeterPos, Meters, MetersPerSec};
use crate::systems::actor::{Action, Actor, ActorModel, appearance, held, set_facing};
use crate::systems::player::{may_command, sender_player};
use crate::systems::stat::{self, StatKind};
use crate::systems::{area, drain_requests, reach};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Position>()
        .replicate_filtered::<MoveTarget, With<HeardMove>>()
        .replicate::<HeardMove>()
        .add_client_message::<MoveRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Meters>,
    /// Counts each time the server put the entity somewhere rather than walked it there, so a client
    /// shows it there at once rather than a walk that never happened.
    pub jumps: Jumps,
}

impl Position {
    pub fn at(pos: Pos<Meters>) -> Position {
        Position {
            pos,
            jumps: Jumps::default(),
        }
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Meters>,
    pub seq: MoveSeq,
}

/// A client numbers its [`MoveRequest`]s in the order it sends them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MoveSeq(pub u32);

/// The latest [`MoveRequest`] the server has heard from a player, heeded or not, so its client can
/// tell when the server's walk accounts for every one it sent.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct HeardMove(pub MoveSeq);

pub fn position(world: &World, entity: Entity) -> Option<Pos<Meters>> {
    world.get::<Position>(entity).map(|p| p.pos)
}

pub struct MovementRenderPlugin;

impl Plugin for MovementRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InterpolatePlugin::<RenderPosition>::default())
            .add_systems(Update, predict::walk_ahead.in_set(RenderValueSystems));
    }
}

/// Played back between replication snapshots, or for the client's own character walked ahead of
/// them, rather than read live, so every client-side depiction of an entity (model, camera, health
/// bar, hitbox, positional audio) reads this, never [`Position`].
#[derive(Component, Clone, Copy, PartialEq)]
pub struct RenderPosition(pub Pos<Meters>);

impl Interpolate for RenderPosition {
    type Source = Position;

    fn sample(position: &Position) -> RenderPosition {
        RenderPosition(position.pos)
    }

    fn interpolate(&self, next: &RenderPosition, t: f32) -> RenderPosition {
        RenderPosition(self.0 + (next.0 - self.0) * t)
    }

    fn jumps(position: &Position) -> Jumps {
        position.jumps
    }
}

/// The corners left to walk, straight from one to the next, stored back-to-front: the next corner
/// is `corners.last()`, so reaching it pops it off the end in O(1) instead of shifting the vec.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub corners: Vec<Pos<Meters>>,
}

impl Path {
    pub fn next(&self) -> Option<Pos<Meters>> {
        self.corners.last().copied()
    }

    pub fn end(&self) -> Option<Pos<Meters>> {
        self.corners.first().copied()
    }

    /// Whether a walk along the path from `at` passes `goal` on its way.
    pub fn passes(&self, at: Pos<Meters>, goal: Pos<Meters>) -> bool {
        self.segment_through(at, goal).is_some()
    }

    /// The path cut short where it passes `goal`, when it does.
    fn through(&self, at: Pos<Meters>, goal: Pos<Meters>) -> Option<Vec<Pos<Meters>>> {
        let passed = self.segment_through(at, goal)?;
        let mut corners = vec![goal];
        corners.extend(self.corners.iter().rev().take(passed).rev());
        Some(corners)
    }

    /// How many corners a walk from `at` reaches before it passes `goal`.
    fn segment_through(&self, at: Pos<Meters>, goal: Pos<Meters>) -> Option<usize> {
        let mut from = at;
        for (passed, &corner) in self.corners.iter().rev().enumerate() {
            if distance_to_segment(from, corner, goal) <= ON_THE_WAY {
                let reached = if goal.distance(corner) <= ON_THE_WAY {
                    passed + 1
                } else {
                    passed
                };
                return Some(reached.min(self.corners.len()));
            }
            from = corner;
        }
        None
    }
}

/// How far from a walk's line a goal may lie and still count as on its way.
const ON_THE_WAY: Meters = Meters(1e-3);

/// Where an actor's walk ends. An actor walks for as long as it has one, along a [`Path`] planned to
/// it whenever the path ends elsewhere. Only a player's goes out, for its own client to follow.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveTarget {
    pub pos: Pos<Meters>,
}

pub fn move_request(world: &mut World) {
    for request in drain_requests::<MoveRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if let Some(mut heard) = world.get_mut::<HeardMove>(player) {
            heard.0 = heard.0.max(request.message.seq);
        }
        if !may_command(world, player) {
            continue;
        }
        reach::drop_intent(world, player);
        goto(world, player, request.message.pos);
    }
}

pub fn relocate(world: &mut World, entity: Entity, pos: Pos<Meters>) {
    world.entity_mut(entity).remove::<(MoveTarget, Path)>();
    if let Some(mut position) = world.get_mut::<Position>(entity) {
        position.pos = pos;
        position.jumps = position.jumps.next();
        spots::claim(world, entity, pos);
    }
}

/// Stops the actor's walk where it stands.
pub fn halt(world: &mut World, entity: Entity) {
    world.entity_mut(entity).remove::<(MoveTarget, Path)>();
    if let Some(at) = position(world, entity) {
        spots::claim(world, entity, at);
    }
}

pub fn moving(world: &World, entity: Entity) -> bool {
    world.get::<MoveTarget>(entity).is_some()
}

/// Sends the actor walking to `pos`, exactly there, and takes the spot. A walk already headed there
/// carries on untouched, and one heading elsewhere turns toward it from where it is.
pub fn goto(world: &mut World, entity: Entity, pos: Pos<Meters>) {
    if world
        .get::<MoveTarget>(entity)
        .is_none_or(|target| target.pos != pos)
    {
        world.entity_mut(entity).insert(MoveTarget { pos });
        spots::claim(world, entity, pos);
    }
}

type Movers = QueryState<Entity, With<MoveTarget>>;
type StepQuery = QueryState<(
    &'static mut Position,
    &'static mut Path,
    Option<&'static mut Actor>,
)>;

pub fn advance(world: &mut World, movers_query: &mut Movers, step_query: &mut StepQuery) {
    let dt = Seconds(world.resource::<Time>().delta_secs());
    let movers: Vec<Entity> = movers_query.iter(world).collect();
    for id in movers {
        if stat::is_dead(world, id) {
            halt(world, id);
            continue;
        }
        let Some(goal) = world.get::<MoveTarget>(id).map(|target| target.pos) else {
            continue;
        };
        let planned = world
            .get::<Path>(id)
            .is_some_and(|path| path.end() == Some(goal));
        if !planned {
            match plan(world, id, goal) {
                Some(corners) => {
                    world.entity_mut(id).insert(Path { corners });
                }
                None => {
                    halt(world, id);
                    continue;
                }
            }
        }

        let speed = speed_of(world, id);
        let gait = gait_of(world, id, speed);
        let Ok((mut position, mut path, mut actor)) = step_query.get_mut(world, id) else {
            continue;
        };
        let mut at = position.pos;
        let heading = walk(&mut at, &mut path, speed * dt);
        let arrived = path.corners.is_empty();
        position.pos = at;
        if let Some(step) = heading
            && let Some(actor) = actor.as_mut()
        {
            set_facing(actor, Radians::of(step), gait);
            if actor.speed != speed {
                actor.speed = speed;
            }
        }
        if arrived {
            world.entity_mut(id).remove::<(MoveTarget, Path)>();
        }
    }
}

pub fn speed_of(world: &World, entity: Entity) -> MetersPerSec {
    MetersPerSec(Meters(stat::effective(
        world,
        entity,
        StatKind::MovementSpeed,
    )))
}

/// The mesh the actor moves on: its area's ground, or its air for a flyer, the wild one when it is
/// wild.
pub fn mesh_of<'c>(content: &'c Content, world: &World, entity: Entity) -> Option<&'c NavMesh> {
    let area = area::of_in(content, world, entity)?;
    Some(area.mesh_for(area::wild(world, entity), airborne(world, entity)))
}

fn mesh_in(world: &World, entity: Entity) -> Option<&NavMesh> {
    mesh_of(world.resource::<Content>(), world, entity)
}

/// The gait the actor's model covers ground at `speed` in, in the stance it stands in.
pub(super) fn gait_of(world: &World, entity: Entity, speed: MetersPerSec) -> Action {
    appearance::model_of(world, entity).map_or(Action::Walk, |model| {
        model.gait(held::stance_of(world, entity), speed)
    })
}

/// Walks `at` along `path` for `distance`, dropping each corner it reaches. Returns the heading of
/// the last stretch walked, if it walked at all.
fn walk(at: &mut Pos<Meters>, path: &mut Path, distance: Meters) -> Option<Offset<Meters>> {
    let mut heading = None;
    let mut remaining = distance;
    while remaining > Meters(1e-6)
        && let Some(target) = path.next()
    {
        let gap = at.distance(target);
        if gap < Meters(1e-4) {
            *at = target;
            path.corners.pop();
            continue;
        }
        heading = Some(target - *at);
        if gap <= remaining {
            *at = target;
            remaining -= gap;
            path.corners.pop();
        } else {
            *at = at.toward(target, remaining);
            remaining = Meters(0.0);
        }
    }
    heading
}

fn plan(world: &World, entity: Entity, goal: Pos<Meters>) -> Option<Vec<Pos<Meters>>> {
    let at = position(world, entity)?;
    plan_on(mesh_in(world, entity)?, at, world.get::<Path>(entity), goal)
}

/// The corners, back to front, of a walk to `goal` by an actor at `at` walking `path`. A goal the
/// walk already passes keeps the walk as it is up to there; any other is routed afresh from where the
/// actor stands.
fn plan_on(
    mesh: &NavMesh,
    at: Pos<Meters>,
    path: Option<&Path>,
    goal: Pos<Meters>,
) -> Option<Vec<Pos<Meters>>> {
    if let Some(kept) = path.and_then(|path| path.through(at, goal)) {
        return Some(kept);
    }
    let mut corners = mesh.route(at, goal)?;
    corners.reverse();
    Some(corners)
}

fn distance_to_segment(a: Pos<Meters>, b: Pos<Meters>, point: Pos<Meters>) -> Meters {
    let along = b - a;
    let length2 = along.square_length();
    let nearest = if length2 < f32::EPSILON {
        a
    } else {
        a + along * ((point - a).dot(along) / length2).clamp(0.0, 1.0)
    };
    nearest.distance(point)
}

fn airborne(world: &World, entity: Entity) -> bool {
    appearance::model_of(world, entity).is_some_and(ActorModel::airborne)
}
