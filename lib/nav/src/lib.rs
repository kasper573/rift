use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use math::{Pos, Size, Unit};

// A* step costs in fixed-point: one tile = 1000, a diagonal = √2 ≈ 1.414 tiles. Integer costs keep
// pathfinding deterministic (no float), and the 1:√2 ratio makes the octile heuristic exact.
const ORTHOGONAL: i64 = 1000;
const DIAGONAL: i64 = 1414;

/// A walkability grid over unit `U` (the game uses `Grid<Tiles>`). A [`Pos<U>`] selects the integer
/// cell it falls in, so callers pass world positions directly without flooring them first.
#[derive(Clone)]
pub struct Grid<U> {
    size: Size<U>,
    walkable: Vec<bool>,
}

impl<U: Unit> Grid<U> {
    pub fn new(size: Size<U>, walkable: Vec<bool>) -> Grid<U> {
        Grid { size, walkable }
    }

    pub fn size(&self) -> Size<U> {
        self.size
    }

    pub fn walkable(&self, p: Pos<U>) -> bool {
        self.cell_walkable(cell(p))
    }

    /// The nearest walkable cell's position (its lower corner), spiralling outward; `None` if the
    /// whole grid is blocked.
    pub fn nearest_walkable(&self, p: Pos<U>) -> Option<Pos<U>> {
        let from = cell(p);
        if self.cell_walkable(from) {
            return Some(at(from));
        }
        let (width, height) = self.dims();
        for radius in 1..=width.max(height) {
            let mut best: Option<Pos<i32>> = None;
            let mut best_d2 = i64::MAX;
            for ny in (from.y - radius)..=(from.y + radius) {
                for nx in (from.x - radius)..=(from.x + radius) {
                    if (nx - from.x).abs() != radius && (ny - from.y).abs() != radius {
                        continue;
                    }
                    let candidate = Pos::new(nx, ny);
                    let d2 = i64::from(nx - from.x).pow(2) + i64::from(ny - from.y).pow(2);
                    if self.cell_walkable(candidate) && d2 < best_d2 {
                        best_d2 = d2;
                        best = Some(candidate);
                    }
                }
            }
            if let Some(best) = best {
                return Some(at(best));
            }
        }
        None
    }

    fn dims(&self) -> (i32, i32) {
        (self.size.x.raw() as i32, self.size.y.raw() as i32)
    }

    fn cell_walkable(&self, c: Pos<i32>) -> bool {
        let (width, height) = self.dims();
        c.x >= 0
            && c.y >= 0
            && c.x < width
            && c.y < height
            && self.walkable[(c.y * width + c.x) as usize]
    }
}

pub fn astar<U: Unit>(grid: &Grid<U>, start: Pos<U>, goal: Pos<U>) -> Option<Vec<Pos<U>>> {
    let start = cell(start);
    let goal = cell(goal);
    if !grid.cell_walkable(start) || !grid.cell_walkable(goal) {
        return None;
    }
    if start == goal {
        return Some(vec![at(start)]);
    }

    let mut g_score: HashMap<Pos<i32>, i64> = HashMap::new();
    let mut came_from: HashMap<Pos<i32>, Pos<i32>> = HashMap::new();
    let mut open: BinaryHeap<Reverse<(i64, i32, i32)>> = BinaryHeap::new();
    g_score.insert(start, 0);
    open.push(Reverse((heuristic(start, goal), start.x, start.y)));

    while let Some(Reverse((_, cx, cy))) = open.pop() {
        let current = Pos::new(cx, cy);
        if current == goal {
            return Some(reconstruct(&came_from, goal).into_iter().map(at).collect());
        }
        let current_g = g_score[&current];
        for step in NEIGHBOURS {
            let next = Pos::new(current.x + step.x, current.y + step.y);
            if !grid.cell_walkable(next) {
                continue;
            }
            let diagonal = step.x != 0 && step.y != 0;
            let cost = current_g + if diagonal { DIAGONAL } else { ORTHOGONAL };
            if cost < g_score.get(&next).copied().unwrap_or(i64::MAX) {
                came_from.insert(next, current);
                g_score.insert(next, cost);
                open.push(Reverse((cost + heuristic(next, goal), next.x, next.y)));
            }
        }
    }
    None
}

const NEIGHBOURS: [Pos<i32>; 8] = [
    Pos::new(1, 0),
    Pos::new(-1, 0),
    Pos::new(0, 1),
    Pos::new(0, -1),
    Pos::new(1, 1),
    Pos::new(1, -1),
    Pos::new(-1, 1),
    Pos::new(-1, -1),
];

/// The integer cell a position falls in.
fn cell<U: Unit>(p: Pos<U>) -> Pos<i32> {
    Pos::new(p.x.raw().floor() as i32, p.y.raw().floor() as i32)
}

/// A cell's lower-corner position in unit `U`.
fn at<U: Unit>(c: Pos<i32>) -> Pos<U> {
    Pos::new(U::of(c.x as f32), U::of(c.y as f32))
}

fn heuristic(from: Pos<i32>, goal: Pos<i32>) -> i64 {
    let dx = i64::from((from.x - goal.x).abs());
    let dy = i64::from((from.y - goal.y).abs());
    let (min, max) = if dx < dy { (dx, dy) } else { (dy, dx) };
    (max - min) * ORTHOGONAL + min * DIAGONAL
}

fn reconstruct(came_from: &HashMap<Pos<i32>, Pos<i32>>, goal: Pos<i32>) -> Vec<Pos<i32>> {
    let mut path = vec![goal];
    let mut node = goal;
    while let Some(&previous) = came_from.get(&node) {
        path.push(previous);
        node = previous;
    }
    path.reverse();
    path
}
