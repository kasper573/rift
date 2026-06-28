use crate::core::math::Pos;
use crate::core::tiling::{self, Cell, CellPos, GridSize, TilePos, Tiles};

const ORTHOGONAL: u32 = 1000;
const DIAGONAL: u32 = 1414;

#[derive(Clone)]
pub struct Grid {
    size: GridSize,
    walkable: Vec<bool>,
    components: Vec<u32>,
}

impl Grid {
    pub fn new(size: GridSize, walkable: Vec<bool>) -> Grid {
        let components = compute_components(size, &walkable);
        Grid {
            size,
            walkable,
            components,
        }
    }

    pub fn walkable(&self, p: Pos<Tiles>) -> bool {
        self.cell_walkable(p.cell())
    }

    pub fn component(&self, p: Pos<Tiles>) -> Option<u32> {
        self.component_at_cell(p.cell())
    }

    fn component_at_cell(&self, c: CellPos) -> Option<u32> {
        c.index(self.size).and_then(|i| {
            let comp = self.components[i];
            if comp == 0 { None } else { Some(comp) }
        })
    }

    pub fn nearest_walkable(&self, p: Pos<Tiles>) -> Option<Pos<Tiles>> {
        let from = p.cell();
        if self.cell_walkable(from) {
            return Some(from.center());
        }
        let (width, height) = (self.size.width as i32, self.size.height as i32);
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
                return Some(best.center());
            }
        }
        None
    }

    fn cell_walkable(&self, c: CellPos) -> bool {
        c.index(self.size).is_some_and(|i| self.walkable[i])
    }
}

pub fn astar(grid: &Grid, start: Pos<Tiles>, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
    let start = start.cell();
    let goal = goal.cell();
    if !grid.cell_walkable(start) || !grid.cell_walkable(goal) {
        return None;
    }
    let (path, _cost) = pathfinding::prelude::astar(
        &start,
        |&c| {
            tiling::NEIGHBORS_8.iter().filter_map(move |&(dx, dy)| {
                let next = c.step((dx, dy));
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
            for &(dx, dy) in &tiling::NEIGHBORS_8 {
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
