//! Where a ray meets a grid of heights: the ray flies over the grid's blocks from the largest down,
//! passing over every block it stays above, and is met with the ground exactly in the cells it
//! comes down to.

use bevy::math::Vec3;

use super::{GroundGrid, Level, SAMPLES_PER_METER, TILE_PIXELS, TileAt};
use crate::core::units::GROUND_ORIGIN;

impl<T: Level> GroundGrid<T> {
    /// How far along the ray from `origin` heading `direction` it first lies at or under the
    /// heights, as [`GroundGrid::height_at`] reads them between the samples, from `from` to `to`
    /// along it. Past the grid's edge the heights carry on as they stand at the edge.
    pub fn first_under(&self, origin: Vec3, direction: Vec3, from: f32, to: f32) -> Option<f32> {
        if from.is_nan() || to.is_nan() || from > to {
            return None;
        }
        let per_metre = SAMPLES_PER_METER as f32;
        let ray = SampleRay {
            origin: Vec3::new(
                (origin.x - GROUND_ORIGIN.x) * per_metre,
                origin.y,
                (origin.z - GROUND_ORIGIN.y) * per_metre,
            ),
            direction: Vec3::new(
                direction.x * per_metre,
                direction.y,
                direction.z * per_metre,
            ),
        };
        let top = TILE_PIXELS.trailing_zeros();
        let mut level = top;
        let mut t = from;
        let mut node = ray.node(t, level);
        loop {
            let (exit_x, exit_z) = ray.exits(node, level);
            let exit = exit_x.min(exit_z).min(to).max(t);
            if level == 0 {
                if let Some(hit) = self.cell_hit(&ray, node, t, exit) {
                    return Some(hit);
                }
            } else if ray.lowest(t, exit) <= self.block_highest(node, level) {
                level -= 1;
                node = ray.child(t, level, node);
                continue;
            }
            if exit >= to {
                return None;
            }
            t = exit;
            let before = node;
            if exit_x <= exit_z {
                node.0 += ray.direction.x.signum() as i64;
            } else {
                node.1 += ray.direction.z.signum() as i64;
            }
            let mut left = before;
            while level < top && (left.0 >> 1, left.1 >> 1) != (node.0 >> 1, node.1 >> 1) {
                left = (left.0 >> 1, left.1 >> 1);
                node = (node.0 >> 1, node.1 >> 1);
                level += 1;
            }
        }
    }

    /// At least the highest of the samples from `low` to `high`, both included, as
    /// [`GroundGrid::index`] reads them: a loaded tile's own, the overview's for one not loaded.
    fn highest_in(&self, low: (u32, u32), high: (u32, u32)) -> T {
        let last = (
            self.grid.columns.saturating_sub(1),
            self.grid.rows.saturating_sub(1),
        );
        let high = (high.0.min(last.0), high.1.min(last.1));
        let low = (low.0.min(high.0), low.1.min(high.1));
        let (first_tile, last_tile) = (
            TileAt::holding(low.0, low.1),
            TileAt::holding(high.0, high.1),
        );
        let mut highest = None;
        for row in first_tile.row..=last_tile.row {
            for column in first_tile.column..=last_tile.column {
                let at = TileAt { column, row };
                let first = self.grid.first(at);
                let held_low = (low.0.max(first.0), low.1.max(first.1));
                let held_high = (
                    high.0.min(first.0 + TILE_PIXELS - 1),
                    high.1.min(first.1 + TILE_PIXELS - 1),
                );
                let tile = self
                    .tiles
                    .get(self.grid.index(at))
                    .and_then(|slot| slot.tile());
                let value = match tile {
                    Some(tile) => tile.highest(
                        (held_low.0 - first.0, held_low.1 - first.1),
                        (held_high.0 - first.0, held_high.1 - first.1),
                    ),
                    None => self.overview_highest(held_low, held_high),
                };
                highest = highest.max(Some(value));
            }
        }
        highest.unwrap_or_else(|| self.blank_value())
    }

    /// At least the highest the overview answers for the samples from `low` to `high`.
    fn overview_highest(&self, low: (u32, u32), high: (u32, u32)) -> T {
        match &self.overview {
            Some(overview) => overview.highest(
                self.grid.overview_nearest(low.0, low.1),
                self.grid.overview_nearest(high.0, high.1),
            ),
            None => self.blank_value(),
        }
    }

    /// The highest the samples of the block `node` at `level` reach, in metres.
    fn block_highest(&self, node: (i64, i64), level: u32) -> f32 {
        let side = 1i64 << level;
        let sample = |at: i64, count: u32| at.clamp(0, i64::from(count.saturating_sub(1))) as u32;
        let (columns, rows) = (self.grid.columns, self.grid.rows);
        let low = (sample(node.0 * side, columns), sample(node.1 * side, rows));
        let high = (
            sample(node.0 * side + side, columns),
            sample(node.1 * side + side, rows),
        );
        self.scale.height_of(self.highest_in(low, high).level()).0
    }

    /// Where along the ray from `from` to `to` it first lies at or under the ground of the cell
    /// `cell`, the heights there lying between its four corners.
    fn cell_hit(&self, ray: &SampleRay, cell: (i64, i64), from: f32, to: f32) -> Option<f32> {
        let sample = |at: i64, count: u32| at.clamp(0, i64::from(count.saturating_sub(1))) as u32;
        let (columns, rows) = (self.grid.columns, self.grid.rows);
        let (left, right) = (sample(cell.0, columns), sample(cell.0 + 1, columns));
        let (top, bottom) = (sample(cell.1, rows), sample(cell.1 + 1, rows));
        let height = |column: u32, row: u32| f64::from(self.sample(column, row).0);
        let corner = height(left, top);
        let along_x = height(right, top) - corner;
        let along_z = height(left, bottom) - corner;
        let twist = height(right, bottom) - corner - along_x - along_z;
        let entry = ray.at(from);
        let (u, v) = (
            f64::from(entry.x) - cell.0 as f64,
            f64::from(entry.z) - cell.1 as f64,
        );
        let (du, dv, dy) = (
            f64::from(ray.direction.x),
            f64::from(ray.direction.z),
            f64::from(ray.direction.y),
        );
        let above = f64::from(entry.y) - corner - along_x * u - along_z * v - twist * u * v;
        if above <= 0.0 {
            return Some(from);
        }
        let falling = dy - along_x * du - along_z * dv - twist * (u * dv + v * du);
        let bending = -twist * du * dv;
        let length = f64::from(to - from);
        first_root(bending, falling, above)
            .filter(|&root| root <= length)
            .map(|root| from + root as f32)
    }
}

/// The ray in the grid's samples: across and down a sample at a time, heights in metres.
struct SampleRay {
    origin: Vec3,
    direction: Vec3,
}

impl SampleRay {
    fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }

    /// The block at `level` the ray is in at `t`, taking a point on a block's edge to be in the
    /// block the ray heads into.
    fn node(&self, t: f32, level: u32) -> (i64, i64) {
        let point = self.at(t);
        let side = (1u64 << level) as f32;
        let along = |at: f32, heading: f32| {
            let blocks = at / side;
            if heading < 0.0 {
                blocks.ceil() as i64 - 1
            } else {
                blocks.floor() as i64
            }
        };
        (
            along(point.x, self.direction.x),
            along(point.z, self.direction.z),
        )
    }

    /// The block at `level` the ray is in at `t`, one of the four `parent` splits into.
    fn child(&self, t: f32, level: u32, parent: (i64, i64)) -> (i64, i64) {
        let (x, z) = self.node(t, level);
        (
            x.clamp(parent.0 * 2, parent.0 * 2 + 1),
            z.clamp(parent.1 * 2, parent.1 * 2 + 1),
        )
    }

    /// How far along the ray it leaves the block `node` at `level` across and down.
    fn exits(&self, node: (i64, i64), level: u32) -> (f32, f32) {
        let side = (1u64 << level) as f32;
        let exit = |block: i64, origin: f32, heading: f32| {
            if heading > 0.0 {
                ((block + 1) as f32 * side - origin) / heading
            } else if heading < 0.0 {
                (block as f32 * side - origin) / heading
            } else {
                f32::INFINITY
            }
        };
        (
            exit(node.0, self.origin.x, self.direction.x),
            exit(node.1, self.origin.z, self.direction.z),
        )
    }

    /// The lowest the ray lies between `from` and `to`.
    fn lowest(&self, from: f32, to: f32) -> f32 {
        self.at(from).y.min(self.at(to).y)
    }
}

/// The smallest root above nothing of `a s² + b s + c`, `c` above nothing.
fn first_root(a: f64, b: f64, c: f64) -> Option<f64> {
    if a.abs() < 1e-12 {
        return (b < 0.0).then(|| -c / b);
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let q = -0.5 * (b + b.signum() * discriminant.sqrt());
    let roots = [q / a, if q == 0.0 { f64::INFINITY } else { c / q }];
    roots
        .into_iter()
        .filter(|root| *root >= 0.0)
        .min_by(f64::total_cmp)
}

// The largest blocks a ray flies over are the tiles themselves, so it reads one tile per block.
const _: () = assert!(TILE_PIXELS.is_power_of_two());
