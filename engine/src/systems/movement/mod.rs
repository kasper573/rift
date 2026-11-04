mod berth;
mod navmesh;
mod predict;
mod spots;

pub use berth::{
    Aim, Approach, ApproachPlan, Berths, approach, place_beside, reachable, relocate_beside,
    settle, vacant_near,
};
pub use navmesh::{Island, NavMesh, SEAM_SNAP, TileLayer, kept_clear};
pub use predict::request_walk;
pub use spots::{Bodies, Spot, Spots, crowding};

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_time::Time;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::chunk::{Chunk, InChunk};
use crate::core::content::{Content, RowCheck};
use crate::core::interpolate::{Interpolate, InterpolatePlugin, Jumps, RenderValueSystems};
use crate::core::math::{Offset, Pos};
use crate::core::par;
use crate::core::tick::{Cached, Planner};
use crate::core::units::{MeterPos, Meters, MetersPerSec, Radians, Seconds};
use crate::systems::actor::{Action, Actor, ActorModel, appearance, held, set_facing};
use crate::systems::area::HeldMesh;
use crate::systems::player::{conn_player, may_command};
use crate::systems::stat::{self, StatKind};
use crate::systems::{GameSettings, area, drain_requests, reach};

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};

    app.replicate::<Position>()
        .replicate_private_while::<MoveTarget, HeardMove>()
        .replicate_private::<HeardMove>()
        .add_client_message::<MoveRequest>();
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

/// Files every entity that moved under the chunk it now stands in.
pub fn file_by_chunk(
    mut commands: Commands,
    moved: Query<(Entity, &Position, Option<&InChunk>), Changed<Position>>,
) {
    for (entity, position, filed) in &moved {
        let chunk = Chunk::at(position.pos);
        if filed.map(|filed| filed.0) != Some(chunk) {
            commands.entity(entity).insert(InChunk(chunk));
        }
    }
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

    /// The path led on to `goal` from a little short of its end, when the goal has drifted by
    /// little beside what is left to walk: the walk keeps its route and is routed afresh only over
    /// its last stretch, twice the drift long, so chasing a goal on the move costs a short route
    /// a time rather than a whole one.
    fn amended(
        &self,
        mesh: &NavMesh,
        at: Pos<Meters>,
        goal: Pos<Meters>,
    ) -> Option<Vec<Pos<Meters>>> {
        let end = self.end()?;
        let drift = end.distance(goal);
        if drift.0 > self.length_from(at).0 * DRIFT_SHARE {
            return None;
        }
        let mut lead = Meters(drift.0 * 2.0);
        let mut from = end;
        for (kept, &corner) in self.corners.iter().enumerate().skip(1) {
            let stretch = from.distance(corner);
            if stretch >= lead {
                let cut = from.toward(corner, lead);
                let mut corners = mesh.route(cut, goal)?;
                corners.reverse();
                corners.push(cut);
                corners.extend(&self.corners[kept..]);
                return Some(corners);
            }
            lead -= stretch;
            from = corner;
        }
        None
    }

    /// What is left to walk from `at`.
    fn length_from(&self, at: Pos<Meters>) -> Meters {
        let mut from = at;
        let mut length = Meters(0.0);
        for &corner in self.corners.iter().rev() {
            length = length + from.distance(corner);
            from = corner;
        }
        length
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
                return Some(reached);
            }
            from = corner;
        }
        None
    }
}

/// How far from a walk's line a goal may lie and still count as on its way.
const ON_THE_WAY: Meters = Meters(1e-3);
/// How far a goal may drift, as a share of what is left to walk, before the walk is routed afresh
/// from where the actor stands: a walk led on to a drifted goal is at most twice the drift longer
/// than the shortest, so this bounds the detour to a fifth of the walk.
const DRIFT_SHARE: f32 = 0.1;

/// Where an actor's walk ends. An actor walks for as long as it has one, along a [`Path`] planned to
/// it whenever the path ends elsewhere. Only a player's goes out, for its own client to follow.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[require(Path)]
pub struct MoveTarget {
    pub pos: Pos<Meters>,
}

pub fn move_request(world: &mut World) {
    for request in drain_requests::<MoveRequest>(world) {
        let Some(player) = conn_player(world, request.client) else {
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

/// Walks every mover its tick's stride: worked out on every core from the world as the tick found
/// it, then written in the order the movers stand.
#[derive(Default)]
pub struct AdvanceWalks {
    movers: Cached<Entity, With<MoveTarget>>,
    walking: Option<QueryState<Movers, With<MoveTarget>>>,
    strides: Vec<MoverStep>,
    /// Each mover's stride, under the mover's index.
    stride_of: Vec<Option<u32>>,
}

type Movers = (
    Entity,
    &'static mut Position,
    &'static mut Path,
    Option<&'static mut Actor>,
);

impl Planner for AdvanceWalks {
    fn prepare(&mut self, world: &mut World) {
        self.movers.ready(world);
        self.walking.get_or_insert_with(|| world.query_filtered());
    }

    fn plan(&mut self, world: &World) {
        let dt = Seconds(world.resource::<Time>().delta_secs());
        let ids: Vec<Entity> = self.movers.query(world).iter().collect();
        self.strides = ids
            .par_iter()
            .with_min_len(par::grain(ids.len()))
            .map(|&id| stride(world, id, dt))
            .collect();
        self.stride_of.clear();
        self.stride_of.resize(world.entities().len() as usize, None);
        for (slot, id) in ids.iter().enumerate() {
            self.stride_of[id.index_u32() as usize] = Some(slot as u32);
        }
    }

    fn apply(&mut self, world: &mut World) {
        let (mut halted, mut arrived) = (Vec::new(), Vec::new());
        let walking = self.walking.as_mut().expect("the movers' query is readied");
        for (id, mut position, mut path, mut actor) in walking.iter_mut(world) {
            let Some(slot) = self
                .stride_of
                .get(id.index_u32() as usize)
                .copied()
                .flatten()
            else {
                continue;
            };
            let MoverStep::Walk {
                at,
                planned,
                reached,
                heading,
                speed,
                gait,
            } = std::mem::replace(&mut self.strides[slot as usize], MoverStep::Halt)
            else {
                halted.push(id);
                continue;
            };
            if let Some(corners) = planned {
                path.corners = corners;
            }
            if reached > 0 {
                let left = path.corners.len() - reached;
                path.corners.truncate(left);
            }
            if position.pos != at {
                position.pos = at;
            }
            if let Some(facing) = heading
                && let Some(actor) = actor.as_mut()
            {
                set_facing(actor, facing, gait);
                if actor.speed != speed {
                    actor.speed = speed;
                }
            }
            if path.corners.is_empty() {
                arrived.push(id);
            }
        }
        for id in arrived {
            world.entity_mut(id).remove::<(MoveTarget, Path)>();
        }
        for id in halted {
            halt(world, id);
        }
    }
}

/// What a mover's tick comes to, worked out from the world as the tick found it.
enum MoverStep {
    /// The walk ends where the mover stands: it fell, or no way leads to its goal.
    Halt,
    Walk {
        at: Pos<Meters>,
        /// The corners of a walk routed afresh this tick, back to front.
        planned: Option<Vec<Pos<Meters>>>,
        /// How many corners of the walk the mover reached this tick.
        reached: usize,
        /// Where the mover faces after the stretch it walked, if it walked at all.
        heading: Option<Radians>,
        speed: MetersPerSec,
        gait: Action,
    },
}

fn stride(world: &World, id: Entity, dt: Seconds) -> MoverStep {
    if stat::is_dead(world, id) {
        return MoverStep::Halt;
    }
    let (Some(goal), Some(mut at)) = (
        world.get::<MoveTarget>(id).map(|goal| goal.pos),
        position(world, id),
    ) else {
        return MoverStep::Halt;
    };
    let held = world.get::<Path>(id);
    let planned = match held {
        Some(path) if path.end() == Some(goal) => None,
        _ => match plan(world, id, goal) {
            Some(corners) => Some(corners),
            None => return MoverStep::Halt,
        },
    };
    let corners = planned
        .as_deref()
        .or_else(|| held.map(|path| path.corners.as_slice()))
        .unwrap_or_default();
    let speed = speed_of(world, id);
    let gait = gait_of(world, id, speed);
    let (reached, heading) = walk(&mut at, corners, speed * dt);
    MoverStep::Walk {
        at,
        planned,
        reached,
        heading: heading.map(Radians::of),
        speed,
        gait,
    }
}

/// How fast the actor moves: its pace, slowed by the water it wades through.
pub fn speed_of(world: &World, entity: Entity) -> MetersPerSec {
    let pace = stat::effective(world, entity, StatKind::MovementSpeed);
    MetersPerSec(Meters(pace * wading_share(world, entity)))
}

/// How water slows whoever wades through it: not at all at its edge, easing to `slowest` of their
/// pace where it stands `deepest`, and no slower past it.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wading {
    pub deepest: Meters,
    pub slowest: f32,
}

impl Wading {
    /// The share of their pace a wader keeps in water `depth` deep.
    pub fn share_at(&self, depth: Meters) -> f32 {
        let deep = (depth.0 / self.deepest.0).clamp(0.0, 1.0);
        1.0 - (1.0 - self.slowest) * deep
    }

    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/deepest"),
            self.deepest.0,
            0.01,
            100.0,
            "meters",
        );
        cx.range(
            format!("{pointer}/slowest"),
            self.slowest,
            0.01,
            1.0,
            "of a wader's pace",
        );
    }
}

/// The share of its pace the actor keeps in the water it stands in: all of it on dry ground, in
/// the air, or in a game where water slows no one.
fn wading_share(world: &World, entity: Entity) -> f32 {
    let content = world.resource::<Content>();
    let Some(wading) = content.built::<GameSettings>().wading else {
        return 1.0;
    };
    if airborne(world, entity) {
        return 1.0;
    }
    match (area::of_in(content, world, entity), position(world, entity)) {
        (Some(area), Some(at)) => wading.share_at(area.water.depth_at(&area.heights, at)),
        _ => 1.0,
    }
}

/// The mesh the actor moves on: its area's ground, or its air for a flyer, the wild one when it is
/// wild.
pub fn mesh_of(content: &Content, world: &World, entity: Entity) -> Option<HeldMesh> {
    let area = area::of_in(content, world, entity)?;
    Some(area.mesh_for(area::wild(world, entity), airborne(world, entity)))
}

fn mesh_in(world: &World, entity: Entity) -> Option<HeldMesh> {
    mesh_of(world.resource::<Content>(), world, entity)
}

/// The gait the actor's model covers ground at `speed` in, in the stance it stands in.
pub(super) fn gait_of(world: &World, entity: Entity, speed: MetersPerSec) -> Action {
    appearance::model_of(world, entity).map_or(Action::Walk, |model| {
        model.gait(held::stance_of(world, entity), speed)
    })
}

/// Walks `at` along `corners`, back to front, for `distance`: how many corners it reached, and the
/// heading of the last stretch walked, if it walked at all.
fn walk(
    at: &mut Pos<Meters>,
    corners: &[Pos<Meters>],
    distance: Meters,
) -> (usize, Option<Offset<Meters>>) {
    let mut heading = None;
    let mut remaining = distance;
    let mut reached = 0;
    while remaining > Meters(1e-6)
        && let Some(&target) = corners[..corners.len() - reached].last()
    {
        let gap = at.distance(target);
        if gap < Meters(1e-4) {
            *at = target;
            reached += 1;
            continue;
        }
        heading = Some(target - *at);
        if gap <= remaining {
            *at = target;
            remaining -= gap;
            reached += 1;
        } else {
            *at = at.toward(target, remaining);
            remaining = Meters(0.0);
        }
    }
    (reached, heading)
}

fn plan(world: &World, entity: Entity, goal: Pos<Meters>) -> Option<Vec<Pos<Meters>>> {
    let at = position(world, entity)?;
    let mesh = mesh_in(world, entity)?;
    plan_on(&mesh, at, world.get::<Path>(entity), goal)
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
    if let Some(led_on) = path.and_then(|path| path.amended(mesh, at, goal)) {
        return Some(led_on);
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
