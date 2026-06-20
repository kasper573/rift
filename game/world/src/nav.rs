//! Pathfinding with fixed-point costs for determinism.

use crate::math::{self, CellPos, Pos, Size, Tiles};

const ORTHOGONAL: u32 = 1000;
const DIAGONAL: u32 = 1414;

#[derive(Clone)]
pub struct Grid {
    size: Size<Tiles>,
    walkable: Vec<bool>,
}

impl Grid {
    pub fn new(size: Size<Tiles>, walkable: Vec<bool>) -> Grid {
        Grid { size, walkable }
    }

    pub fn size(&self) -> Size<Tiles> {
        self.size
    }

    pub fn walkable(&self, p: Pos<Tiles>) -> bool {
        self.cell_walkable(math::tile_at(p))
    }

    pub fn nearest_walkable(&self, p: Pos<Tiles>) -> Option<Pos<Tiles>> {
        let from = math::tile_at(p);
        if self.cell_walkable(from) {
            return Some(math::tile_center(from));
        }
        let (width, height) = self.dims();
        for radius in 1..=width.max(height) {
            let mut best: Option<CellPos> = None;
            let mut best_d2 = i64::MAX;
            for ny in (from.y - radius)..=(from.y + radius) {
                for nx in (from.x - radius)..=(from.x + radius) {
                    if (nx - from.x).abs() != radius && (ny - from.y).abs() != radius {
                        continue;
                    }
                    let d2 = i64::from(nx - from.x).pow(2) + i64::from(ny - from.y).pow(2);
                    if self.cell_walkable(CellPos::new(nx, ny)) && d2 < best_d2 {
                        best_d2 = d2;
                        best = Some(CellPos::new(nx, ny));
                    }
                }
            }
            if let Some(best) = best {
                return Some(math::tile_center(best));
            }
        }
        None
    }

    fn dims(&self) -> (i32, i32) {
        (self.size.width as i32, self.size.height as i32)
    }

    fn cell_walkable(&self, c: CellPos) -> bool {
        let (width, height) = self.dims();
        c.x >= 0
            && c.y >= 0
            && c.x < width
            && c.y < height
            && self.walkable[(c.y * width + c.x) as usize]
    }
}

pub fn astar(grid: &Grid, start: Pos<Tiles>, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
    let start = math::tile_at(start);
    let goal = math::tile_at(goal);
    if !grid.cell_walkable(start) || !grid.cell_walkable(goal) {
        return None;
    }
    let (path, _cost) = pathfinding::prelude::astar(
        &start,
        |&c| {
            NEIGHBOURS.iter().filter_map(move |&(dx, dy)| {
                let next = CellPos::new(c.x + dx, c.y + dy);
                let cost = if dx != 0 && dy != 0 {
                    DIAGONAL
                } else {
                    ORTHOGONAL
                };
                grid.cell_walkable(next).then_some((next, cost))
            })
        },
        |&c| {
            let dx = (c.x - goal.x).unsigned_abs();
            let dy = (c.y - goal.y).unsigned_abs();
            (dx.max(dy) - dx.min(dy)) * ORTHOGONAL + dx.min(dy) * DIAGONAL
        },
        |&node| node == goal,
    )?;
    Some(path)
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];
