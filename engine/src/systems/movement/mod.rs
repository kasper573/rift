use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::interpolate::{Interpolate, InterpolatePlugin, Jumps, RenderValueSystems};
use crate::core::math::{Direction, Offset, Pos, Rect, Rng, Size};
use crate::core::tiling::{
    Cell, CellPos, GridDims, GridSize, NEIGHBORS_8, TilePos, TileRect, Tiles, TilesPerSec,
};
use crate::core::time::Seconds;
use crate::systems::actor::{Action, Actor, ActorModel, set_facing};
use crate::systems::player::{may_command, sender_player};
use crate::systems::reach;
use crate::systems::stat::{self, StatKind};
use crate::systems::{area, drain_requests};

mod predict;

pub use predict::request_walk;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Position>()
        .replicate_filtered::<MoveTarget, With<HeardMove>>()
        .replicate::<HeardMove>()
        .add_client_message::<MoveRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Tiles>,
    /// Counts each time the server put the entity somewhere rather than walked it there, so a client
    /// shows it there at once rather than a walk that never happened.
    pub jumps: Jumps,
}

impl Position {
    pub fn at(pos: Pos<Tiles>) -> Position {
        Position {
            pos,
            jumps: Jumps::default(),
        }
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Tiles>,
    pub seq: MoveSeq,
}

/// A client numbers its [`MoveRequest`]s in the order it sends them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MoveSeq(pub u32);

/// The latest [`MoveRequest`] the server has heard from a player, heeded or not, so its client can
/// tell when the server's walk accounts for every one it sent.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct HeardMove(pub MoveSeq);

pub fn position(world: &World, entity: Entity) -> Option<Pos<Tiles>> {
    world.get::<Position>(entity).map(|p| p.pos)
}

pub struct MovementPlugin;

impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InterpolatePlugin::<RenderPosition>::default())
            .add_systems(Update, predict::walk_ahead.in_set(RenderValueSystems));
    }
}

/// Played back between replication snapshots, or for the client's own character walked ahead of
/// them, rather than read live, so every client-side depiction of an entity (sprite, camera, health
/// bar, hitbox, positional audio) reads this, never [`Position`].
#[derive(Component, Clone, Copy, PartialEq)]
pub struct RenderPosition(pub Pos<Tiles>);

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

const RUN_SPEED: TilesPerSec = TilesPerSec(Tiles(2.0));

/// Remaining waypoints, stored back-to-front: the next cell to walk is `tiles.last()`, so advancing
/// pops it off the end in O(1) instead of shifting the whole vec from the front.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub tiles: Vec<CellPos>,
}

impl Path {
    pub fn next(&self) -> Option<CellPos> {
        self.tiles.last().copied()
    }

    pub fn end(&self) -> Option<CellPos> {
        self.tiles.first().copied()
    }
}

/// Where an actor's walk ends. An actor walks for as long as it has one, along a [`Path`] planned to
/// it whenever the path ends elsewhere. Only a player's goes out, for its own client to follow.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveTarget {
    pub pos: Pos<Tiles>,
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

pub fn relocate(world: &mut World, entity: Entity, pos: Pos<Tiles>) {
    world.entity_mut(entity).remove::<(MoveTarget, Path)>();
    if let Some(mut position) = world.get_mut::<Position>(entity) {
        position.pos = pos;
        position.jumps = position.jumps.next();
    }
}

/// Stops the actor's walk at the tile it stands on, or mid-step, at the end of the step.
pub fn halt(world: &mut World, entity: Entity) {
    let Some(mut at) = position(world, entity) else {
        return;
    };
    let mut path = world.entity_mut(entity).take::<Path>().unwrap_or_default();
    let turn = halt_walk(&mut at, &mut path);
    if position(world, entity) != Some(at)
        && let Some(mut position) = world.get_mut::<Position>(entity)
    {
        position.pos = at;
    }
    match turn {
        Some(next) => {
            world.entity_mut(entity).insert(path);
            goto(world, entity, next);
        }
        None => {
            world.entity_mut(entity).remove::<MoveTarget>();
        }
    }
}

pub fn moving(world: &World, entity: Entity) -> bool {
    world.get::<MoveTarget>(entity).is_some()
}

pub fn on_tile(world: &World, entity: Entity) -> bool {
    position(world, entity).is_some_and(|p| p.on_center())
}

/// Sends the actor walking to `pos`. A walk already headed there carries on untouched, and one heading
/// elsewhere turns toward it without dropping the step it's on.
pub fn goto(world: &mut World, entity: Entity, pos: Pos<Tiles>) {
    if world
        .get::<MoveTarget>(entity)
        .is_none_or(|target| target.pos != pos)
    {
        world.entity_mut(entity).insert(MoveTarget { pos });
    }
}

pub fn approach(world: &mut World, entity: Entity, target: Pos<Tiles>, range: Tiles) -> bool {
    let Some(at) = position(world, entity) else {
        return false;
    };
    if at.distance(target) <= range {
        return true;
    }
    let heading = world
        .get::<MoveTarget>(entity)
        .is_some_and(|goal| goal.pos.distance(target) <= range);
    if !heading {
        let dest = approach_tile(world, entity, at, target, range).unwrap_or(target);
        goto(world, entity, dest);
    }
    false
}

fn approach_tile(
    world: &World,
    entity: Entity,
    from: Pos<Tiles>,
    target: Pos<Tiles>,
    range: Tiles,
) -> Option<Pos<Tiles>> {
    let navigator = Navigator::of(world, entity)?;
    let start = from.cell();
    let goal = target.cell();
    let reach = range.0.ceil() as i32;
    let mut best: Option<(Pos<Tiles>, Tiles)> = None;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let cell = goal.step((dx, dy));
            if cell == goal {
                continue;
            }
            let center = cell.center();
            if center.distance(target) > range || !navigator.grid.connects(start, cell) {
                continue;
            }
            let distance = from.distance(center);
            if best.is_none_or(|(_, best)| distance < best) {
                best = Some((center, distance));
            }
        }
    }
    best.map(|(center, _)| center)
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
            world.entity_mut(id).remove::<(MoveTarget, Path)>();
            continue;
        }
        let Some(goal) = world.get::<MoveTarget>(id).map(|target| target.pos) else {
            continue;
        };
        let planned = world
            .get::<Path>(id)
            .is_some_and(|path| path.end() == Some(goal.cell()));
        if !planned {
            match plan(world, id, goal) {
                Some(tiles) => {
                    world.entity_mut(id).insert(Path { tiles });
                }
                None => {
                    halt(world, id);
                    if !world.entity(id).contains::<Path>() {
                        continue;
                    }
                }
            }
        }

        let speed = speed_of(world, id);
        let Ok((mut position, mut path, mut actor)) = step_query.get_mut(world, id) else {
            continue;
        };
        let mut at = position.pos;
        let heading = walk(&mut at, &mut path, speed * dt);
        let arrived = path.tiles.is_empty();
        position.pos = at;
        if let Some(step) = heading
            && let Some(actor) = actor.as_mut()
        {
            set_facing(actor, Direction::from(step), gait(speed));
        }
        if arrived {
            world.entity_mut(id).remove::<(MoveTarget, Path)>();
        }
    }
}

fn speed_of(world: &World, entity: Entity) -> TilesPerSec {
    TilesPerSec(Tiles(stat::effective(
        world,
        entity,
        StatKind::MovementSpeed,
    )))
}

fn gait(speed: TilesPerSec) -> Action {
    if speed >= RUN_SPEED {
        Action::Run
    } else {
        Action::Walk
    }
}

/// Walks `at` along `path` for `distance`, dropping each waypoint it reaches. Returns the heading of
/// the last stretch walked, if it walked at all.
fn walk(at: &mut Pos<Tiles>, path: &mut Path, distance: Tiles) -> Option<Offset<Tiles>> {
    let mut heading = None;
    let mut remaining = distance;
    while remaining > Tiles(1e-6)
        && let Some(next) = path.next()
    {
        let target = next.center();
        let gap = at.distance(target);
        if gap < Tiles(1e-4) {
            *at = target;
            path.tiles.pop();
            continue;
        }
        heading = Some(target - *at);
        if gap <= remaining {
            *at = target;
            remaining -= gap;
            path.tiles.pop();
        } else {
            *at = at.toward(target, remaining);
            remaining = Tiles(0.0);
        }
    }
    heading
}

fn plan(world: &World, entity: Entity, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
    let at = position(world, entity)?;
    Navigator::of(world, entity)?.plan(at, world.get::<Path>(entity), goal)
}

const ORTHOGONAL: u64 = 1000;
const DIAGONAL: u64 = 1414;
const FLIGHT_SAMPLE: Tiles = Tiles(0.25);

/// Route costs are scaled by this, leaving the bits below it to break ties between equally cheap
/// routes without ever outweighing a real difference in cost.
const TIE: u64 = 1 << 20;

/// How much a tile of distance from the straight line to the goal counts toward breaking ties.
const LEAN_PER_TILE: f32 = 1024.0;

#[derive(Clone)]
pub struct Grid {
    size: GridSize,
    walkable: Vec<bool>,
    components: Vec<u32>,
    nodes: Vec<Pos<Tiles>>,
    nodes_by_component: Vec<Vec<Pos<Tiles>>>,
}

impl Grid {
    pub fn new(size: GridSize, walkable: Vec<bool>) -> Grid {
        let components = compute_components(size, &walkable);
        let mut nodes = Vec::new();
        let mut nodes_by_component: Vec<Vec<Pos<Tiles>>> = Vec::new();
        for (index, cell) in size.cells().enumerate() {
            let component = components[index] as usize;
            if component == 0 {
                continue;
            }
            if component >= nodes_by_component.len() {
                nodes_by_component.resize(component + 1, Vec::new());
            }
            nodes.push(cell.center());
            nodes_by_component[component].push(cell.center());
        }
        Grid {
            size,
            walkable,
            components,
            nodes,
            nodes_by_component,
        }
    }

    pub fn walkable(&self, p: Pos<Tiles>) -> bool {
        self.cell_walkable(p.cell())
    }

    pub fn component(&self, p: Pos<Tiles>) -> Option<u32> {
        self.component_at_cell(p.cell())
    }

    pub fn nodes(&self) -> &[Pos<Tiles>] {
        &self.nodes
    }

    pub fn random_node(&self, rng: &mut Rng) -> Option<Pos<Tiles>> {
        pick(rng, &self.nodes)
    }

    pub fn random_reachable(&self, rng: &mut Rng, from: Pos<Tiles>) -> Option<Pos<Tiles>> {
        let component = self.component(from)? as usize;
        pick(rng, self.nodes_by_component.get(component)?)
    }

    pub fn random_node_within(&self, rng: &mut Rng, bounds: Rect<Tiles>) -> Option<Pos<Tiles>> {
        pick(rng, &within(&self.nodes, bounds))
    }

    pub fn random_reachable_within(
        &self,
        rng: &mut Rng,
        from: Pos<Tiles>,
        bounds: Rect<Tiles>,
    ) -> Option<Pos<Tiles>> {
        let nodes = bounds.tiles().map(Cell::center);
        self.random_reachable_among(rng, from, nodes.filter(|&node| bounds.contains(node)))
    }

    pub fn random_reachable_near(
        &self,
        rng: &mut Rng,
        from: Pos<Tiles>,
        center: Pos<Tiles>,
        radius: Tiles,
    ) -> Option<Pos<Tiles>> {
        let square = Rect::new(
            center - Offset::new(radius.0, radius.0),
            Size::new(2.0 * radius.0, 2.0 * radius.0),
        );
        let nodes = square.tiles().map(Cell::center);
        self.random_reachable_among(
            rng,
            from,
            nodes.filter(|&node| node.distance(center) <= radius),
        )
    }

    fn random_reachable_among(
        &self,
        rng: &mut Rng,
        from: Pos<Tiles>,
        nodes: impl Iterator<Item = Pos<Tiles>>,
    ) -> Option<Pos<Tiles>> {
        let component = self.component(from)?;
        let reachable: Vec<Pos<Tiles>> = nodes
            .filter(|&node| self.component(node) == Some(component))
            .collect();
        pick(rng, &reachable)
    }

    fn component_at_cell(&self, c: CellPos) -> Option<u32> {
        c.index(self.size).and_then(|i| {
            let comp = self.components[i];
            if comp == 0 { None } else { Some(comp) }
        })
    }

    /// The cheapest walk between two cells, both ends included.
    pub fn path(&self, start: CellPos, goal: CellPos) -> Option<Vec<CellPos>> {
        self.route(start.center(), &[(start, Tiles(0.0))], goal)
    }

    /// The cheapest walk to `goal` for an actor at `at` setting off from one of `footing`'s cells,
    /// each that far from it; the walk starts with the cell it sets off from. Of equally cheap walks
    /// it takes the one keeping closest to the straight line from `at` to the goal, so a walk looks
    /// straight wherever the grid allows, and a walk continued along that line stays on it.
    pub fn route(
        &self,
        at: Pos<Tiles>,
        footing: &[(CellPos, Tiles)],
        goal: CellPos,
    ) -> Option<Vec<CellPos>> {
        if !footing.iter().any(|&(cell, _)| self.connects(cell, goal)) {
            return None;
        }
        let line = goal.center() - at;
        let length = line.length();
        let lean = |cell: CellPos| {
            if length < 1e-6 {
                return 0;
            }
            let off = cell.center() - at;
            let distance = (off.x * line.y - off.y * line.x).abs() / length;
            ((distance * LEAN_PER_TILE) as u64).min(TIE - 1)
        };
        let (path, _cost) = pathfinding::prelude::astar(
            &None,
            |&node: &Option<CellPos>| {
                let setting_off =
                    footing
                        .iter()
                        .filter(move |_| node.is_none())
                        .map(|&(cell, distance)| {
                            let cost = (distance.0 * ORTHOGONAL as f32).round() as u64;
                            (Some(cell), cost * TIE)
                        });
                let stepping = node.into_iter().flat_map(move |cell| {
                    NEIGHBORS_8.iter().filter_map(move |&(dx, dy)| {
                        let next = cell.step((dx, dy));
                        let cost = if dx != 0 && dy != 0 {
                            DIAGONAL
                        } else {
                            ORTHOGONAL
                        };
                        self.cell_walkable(next).then_some((Some(next), cost * TIE))
                    })
                });
                setting_off.chain(stepping)
            },
            |&node| node.map_or(0, |cell| octile(cell, goal) * TIE + lean(cell)),
            |&node| node == Some(goal),
        )?;
        Some(path.into_iter().flatten().collect())
    }

    pub fn nearest_walkable(&self, p: Pos<Tiles>) -> Option<Pos<Tiles>> {
        self.nearest_walkable_where(p, |_| true)
    }

    /// The walkable cell nearest `p` that `fits`, `p`'s own when it does. Of equally near cells the
    /// southernmost wins, then the westernmost: an actor put down beside another is drawn in front
    /// of it, not hidden behind it.
    pub fn nearest_walkable_where(
        &self,
        p: Pos<Tiles>,
        fits: impl Fn(CellPos) -> bool,
    ) -> Option<Pos<Tiles>> {
        let from = p.cell();
        if self.cell_walkable(from) && fits(from) {
            return Some(from.center());
        }
        let (width, height) = (self.size.width as i32, self.size.height as i32);
        for radius in 1..=width.max(height) {
            let mut best: Option<CellPos> = None;
            let mut best_d2 = i64::MAX;
            for ny in ((from.y - radius)..=(from.y + radius)).rev() {
                for nx in (from.x - radius)..=(from.x + radius) {
                    if (nx - from.x).abs() != radius && (ny - from.y).abs() != radius {
                        continue;
                    }
                    let d2 = i64::from(nx - from.x).pow(2) + i64::from(ny - from.y).pow(2);
                    let cell = CellPos::new(nx, ny);
                    if self.cell_walkable(cell) && fits(cell) && d2 < best_d2 {
                        best_d2 = d2;
                        best = Some(cell);
                    }
                }
            }
            if let Some(best) = best {
                return Some(best.center());
            }
        }
        None
    }

    fn cell_walkable(&self, c: CellPos) -> bool {
        c.index(self.size).is_some_and(|i| self.walkable[i])
    }

    fn connects(&self, from: CellPos, to: CellPos) -> bool {
        self.component_at_cell(from)
            .is_some_and(|component| self.component_at_cell(to) == Some(component))
    }
}

struct Navigator<'c> {
    grid: &'c Grid,
    flies: bool,
}

impl<'c> Navigator<'c> {
    fn of(world: &'c World, entity: Entity) -> Option<Navigator<'c>> {
        Navigator::of_in(world.resource::<Content>(), world, entity)
    }

    fn of_in(content: &'c Content, world: &World, entity: Entity) -> Option<Navigator<'c>> {
        let area = area::of_in(content, world, entity)?;
        let wild = area::wild(world, entity);
        let flies = airborne(world, entity);
        Some(Navigator {
            grid: if flies {
                area.airspace_for(wild)
            } else {
                area.grid_for(wild)
            },
            flies,
        })
    }

    /// The waypoints, back to front, of a walk to `goal` by an actor at `at` walking `path`. A goal
    /// the walk already passes keeps the walk as it is up to there; any other is routed afresh from
    /// where the actor stands.
    fn plan(&self, at: Pos<Tiles>, path: Option<&Path>, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
        let end = goal.cell();
        if let Some(path) = path
            && let Some(on_the_way) = path.tiles.iter().position(|&cell| cell == end)
        {
            return Some(path.tiles[on_the_way..].to_vec());
        }
        if self.flies && self.clear(at, goal) {
            return Some(vec![end]);
        }
        let mut tiles = self.grid.route(at, &self.footing(at, path), end)?;
        if at.on_center() {
            tiles.remove(0);
        }
        tiles.reverse();
        Some(tiles)
    }

    /// The cells an actor at `at` can set off from, each with how far it is: the one it stands on,
    /// or mid-step on foot, both ends of the step, so it can carry on or turn straight back.
    fn footing(&self, at: Pos<Tiles>, path: Option<&Path>) -> Vec<(CellPos, Tiles)> {
        let here = at.cell();
        let Some(next) = path
            .and_then(Path::next)
            .filter(|_| !self.flies && !at.on_center())
        else {
            return vec![(here, at.distance(here.center()))];
        };
        [next, stepping_off(at, next)]
            .into_iter()
            .filter(|&cell| self.grid.cell_walkable(cell))
            .map(|cell| (cell, at.distance(cell.center())))
            .collect()
    }

    fn clear(&self, from: Pos<Tiles>, to: Pos<Tiles>) -> bool {
        let samples = (from.distance(to).0 / FLIGHT_SAMPLE.0).ceil().max(1.0) as u32;
        (0..=samples).all(|i| {
            self.grid
                .cell_walkable(from.lerp(to, i as f32 / samples as f32).cell())
        })
    }
}

/// Stops a walk at `at`: at the end of the step it's on if it's mid-step, where it turns to the
/// returned goal, or else on the tile it stands on, where the walk ends.
fn halt_walk(at: &mut Pos<Tiles>, path: &mut Path) -> Option<Pos<Tiles>> {
    match path.next().filter(|_| !at.on_center()) {
        Some(next) => {
            *path = Path { tiles: vec![next] };
            Some(next.center())
        }
        None => {
            path.tiles.clear();
            if at.on_center() {
                *at = at.snap();
            }
            None
        }
    }
}

/// The cell an actor at `at`, mid-step toward `next`, is stepping off.
fn stepping_off(at: Pos<Tiles>, next: CellPos) -> CellPos {
    let toward = |cell: i32, from: f32| {
        let gap = cell as f32 - from;
        if gap > 1e-4 {
            1
        } else if gap < -1e-4 {
            -1
        } else {
            0
        }
    };
    CellPos::new(next.x - toward(next.x, at.x), next.y - toward(next.y, at.y))
}

fn octile(from: CellPos, to: CellPos) -> u64 {
    let dx = u64::from((from.x - to.x).unsigned_abs());
    let dy = u64::from((from.y - to.y).unsigned_abs());
    (dx.max(dy) - dx.min(dy)) * ORTHOGONAL + dx.min(dy) * DIAGONAL
}

fn airborne(world: &World, entity: Entity) -> bool {
    let content = world.resource::<Content>();
    world
        .get::<Actor>(entity)
        .is_some_and(|actor| ActorModel::get(content, actor.model).airborne())
}

fn within(nodes: &[Pos<Tiles>], bounds: Rect<Tiles>) -> Vec<Pos<Tiles>> {
    nodes
        .iter()
        .copied()
        .filter(|&node| bounds.contains(node))
        .collect()
}

fn pick(rng: &mut Rng, nodes: &[Pos<Tiles>]) -> Option<Pos<Tiles>> {
    if nodes.is_empty() {
        return None;
    }
    Some(nodes[rng.rand_range(0..nodes.len() as u32) as usize])
}

fn compute_components(size: GridSize, walkable: &[bool]) -> Vec<u32> {
    let mut components = vec![0u32; walkable.len()];
    let mut next_id = 1u32;

    for (i, &is_walkable) in walkable.iter().enumerate() {
        if !is_walkable || components[i] != 0 {
            continue;
        }

        let start_cell = CellPos::new(
            (i % size.width as usize) as i32,
            (i / size.width as usize) as i32,
        );
        let comp_id = next_id;
        next_id += 1;

        let mut queue = vec![start_cell];
        components[i] = comp_id;

        while let Some(c) = queue.pop() {
            for &(dx, dy) in &NEIGHBORS_8 {
                let next = c.step((dx, dy));
                if let Some(next_i) = next.index(size)
                    && walkable[next_i]
                    && components[next_i] == 0
                {
                    components[next_i] = comp_id;
                    queue.push(next);
                }
            }
        }
    }

    components
}
