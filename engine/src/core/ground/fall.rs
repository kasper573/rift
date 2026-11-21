//! Which way water runs: how its surface falls along itself, read across the steps its levels
//! are kept in and round the bends of its banks, so the gentlest river tells its way downstream.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy::math::Vec2;

use crate::core::units::Meters;

/// How the water of a lattice of points falls, read once for the whole lattice and asked of each
/// point.
pub struct WaterFalls {
    cells: Cells,
    /// How many points across a cell is.
    side: usize,
    /// How each wet cell falls, none on dry ground; empty where no water of the lattice falls.
    falls: Vec<Option<Vec2>>,
}

impl WaterFalls {
    /// Reads how the water falls on a lattice `columns` across. `levels` say of each point, row
    /// by row, how high the water stands there, in metres, none on dry ground; the points lie
    /// `spacing` apart. A level is kept in steps, so a gentle river stands as a flight of flat
    /// reaches: its surface is taken to slope evenly along each reach, the way the water runs
    /// through it, from the step above it down to the step below, sought as far along the water
    /// as [`FALL_REACH`] and never across dry ground. A river so falls evenly along its whole
    /// run, along its banks, and still water doesn't fall at all.
    pub fn read(levels: &[Option<f32>], columns: usize, spacing: Meters) -> WaterFalls {
        let rows = levels.len().checked_div(columns).unwrap_or(0);
        let side = fall_cell(spacing);
        let cells = Cells {
            across: columns.div_ceil(side),
            down: rows.div_ceil(side),
        };
        let (lowest, highest) = levels
            .iter()
            .flatten()
            .fold((f32::MAX, f32::MIN), |(lowest, highest), level| {
                (lowest.min(*level), highest.max(*level))
            });
        if highest - lowest <= SAME_REACH {
            return WaterFalls {
                cells,
                side,
                falls: Vec::new(),
            };
        }
        let stands: Vec<Option<f32>> = (0..cells.across * cells.down)
            .map(|at| {
                let (first_column, first_row) =
                    (at % cells.across * side, at / cells.across * side);
                // A cell stands as high as its highest point, never between two reaches.
                (first_row..(first_row + side).min(rows))
                    .flat_map(|row| {
                        (first_column..(first_column + side).min(columns))
                            .filter_map(move |column| levels[row * columns + column])
                    })
                    .reduce(f32::max)
            })
            .collect();
        let reach = fall_reach(spacing) as u32 * STRAIGHT;
        let below = cells.steps(&stands, reach, |here, there| here - there);
        let above = cells.steps(&stands, reach, |here, there| there - here);
        let mut sloped: Vec<Option<f32>> = (0..stands.len())
            .map(|at| {
                let level = stands[at]?;
                let (to_below, to_above) = (
                    below[at].map_or(reach, |step| step.away),
                    above[at].map_or(reach, |step| step.away),
                );
                let Some(rise) = above[at].or(below[at]).map(|step| step.high) else {
                    return Some(level);
                };
                Some(level + rise * to_below as f32 / (to_below + to_above) as f32)
            })
            .collect();
        for _ in 0..FALL_EVENED {
            let before = sloped.clone();
            for (at, level) in sloped.iter_mut().enumerate() {
                let Some(here) = before[at] else {
                    continue;
                };
                let (sum, count) = cells
                    .beside(at)
                    .filter(|(_, cost)| *cost == STRAIGHT)
                    .filter_map(|(there, _)| before[there])
                    .fold((here, 1.0), |(sum, count), level| {
                        (sum + level, count + 1.0)
                    });
                *level = Some(sum / count);
            }
        }
        let cell = spacing.0 * side as f32;
        let falls = (0..sloped.len())
            .map(|at| {
                let here = sloped[at]?;
                let held =
                    |way: (isize, isize)| cells.toward(at, way).and_then(|there| sloped[there]);
                let along = |ahead: (isize, isize), behind: (isize, isize)| match (
                    held(ahead),
                    held(behind),
                ) {
                    (Some(ahead), Some(behind)) => (behind - ahead) / (2.0 * cell),
                    (Some(ahead), None) => (here - ahead) / cell,
                    (None, Some(behind)) => (behind - here) / cell,
                    (None, None) => 0.0,
                };
                Some(Vec2::new(along((1, 0), (-1, 0)), along((0, 1), (0, -1))))
            })
            .collect();
        WaterFalls { cells, side, falls }
    }

    /// How the water falls at the point `column`, `row` of the lattice, in metres a metre toward
    /// east and south. A dry point beside the water falls as the water next to it.
    pub fn at(&self, column: usize, row: usize) -> Vec2 {
        if self.falls.is_empty() {
            return Vec2::ZERO;
        }
        let among = (Vec2::new(column as f32, row as f32) + 0.5) / self.side as f32 - 0.5;
        let first = among.floor();
        let share = among - first;
        let (fall, weight) = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .into_iter()
            .filter_map(|(right, below)| {
                let cell = self
                    .cells
                    .at(first.x as isize + right, first.y as isize + below)?;
                let weight = (if right == 1 { share.x } else { 1.0 - share.x })
                    * (if below == 1 { share.y } else { 1.0 - share.y });
                Some((self.falls[cell]? * weight, weight))
            })
            .fold((Vec2::ZERO, 0.0), |(fall, weight), (held, share)| {
                (fall + held, weight + share)
            });
        if weight > 0.0 {
            fall / weight
        } else {
            Vec2::ZERO
        }
    }
}

/// How many points round a block of the lattice the fall at the block's points is read from, the
/// points lying `spacing` apart: a block read with this margin falls as its neighbours do where
/// they meet.
pub fn fall_margin(spacing: Meters) -> usize {
    (fall_reach(spacing) + FALL_EVENED + 2) * fall_cell(spacing)
}

/// How far along the water the steps above and below a reach are sought.
const FALL_REACH: Meters = Meters(64.0);

/// How wide the cells are that the water is read in: a few across a brook.
const FALL_CELL: Meters = Meters(2.0);

/// How many times the sloped surface is evened out with the cells beside it, which rounds off
/// the corners the grid leaves in it.
const FALL_EVENED: usize = 6;

/// How much lower a cell must stand than the one beside it to lie on another reach, in metres:
/// more than rounding leaves between two cells of one reach.
const SAME_REACH: f32 = 1e-4;

/// What a step straight to the cell beside costs, and one cornerwise.
const STRAIGHT: u32 = 10;
const CORNERWISE: u32 = 14;

/// How many points across a cell is where the points lie `spacing` apart.
fn fall_cell(spacing: Meters) -> usize {
    ((FALL_CELL.0 / spacing.0).round() as usize).max(1)
}

/// How many cells along the water a reach's steps are sought.
fn fall_reach(spacing: Meters) -> usize {
    (FALL_REACH.0 / (spacing.0 * fall_cell(spacing) as f32)).ceil() as usize
}

/// A step in the water's level as a cell on a reach knows it: how far along the reach it lies,
/// in the costs of the way there, and how high it is, in metres.
#[derive(Clone, Copy)]
struct Step {
    away: u32,
    high: f32,
}

/// The cells the water is read in, row by row.
struct Cells {
    across: usize,
    down: usize,
}

impl Cells {
    fn at(&self, column: isize, row: isize) -> Option<usize> {
        (column >= 0 && row >= 0 && (column as usize) < self.across && (row as usize) < self.down)
            .then(|| row as usize * self.across + column as usize)
    }

    fn toward(&self, at: usize, way: (isize, isize)) -> Option<usize> {
        self.at(
            (at % self.across) as isize + way.0,
            (at / self.across) as isize + way.1,
        )
    }

    /// The cells round `at`, each with what the step to it costs.
    fn beside(&self, at: usize) -> impl Iterator<Item = (usize, u32)> + '_ {
        (-1..=1)
            .flat_map(|down| (-1..=1).map(move |across| (across, down)))
            .filter(|way| *way != (0, 0))
            .filter_map(move |way| {
                let cost = if way.0 == 0 || way.1 == 0 {
                    STRAIGHT
                } else {
                    CORNERWISE
                };
                Some((self.toward(at, way)?, cost))
            })
    }

    /// The nearest step each wet cell's reach ends in, no farther along the reach than `reach`:
    /// a step is where the water beside a cell stands `drop(here, there)` off it, by more than
    /// cells of one reach differ.
    fn steps(
        &self,
        stands: &[Option<f32>],
        reach: u32,
        drop: impl Fn(f32, f32) -> f32,
    ) -> Vec<Option<Step>> {
        let mut nearest: Vec<Option<Step>> = vec![None; stands.len()];
        let mut open = BinaryHeap::new();
        for (at, here) in stands.iter().enumerate() {
            let Some(here) = here else {
                continue;
            };
            let step = self
                .beside(at)
                .filter_map(|(there, cost)| Some((drop(*here, stands[there]?), cost)))
                .filter(|(high, _)| *high > SAME_REACH)
                .min_by_key(|(_, cost)| *cost);
            if let Some((high, cost)) = step {
                nearest[at] = Some(Step {
                    away: cost / 2,
                    high,
                });
                open.push(Reverse((cost / 2, at)));
            }
        }
        while let Some(Reverse((away, at))) = open.pop() {
            let Some(step) = nearest[at].filter(|step| step.away == away) else {
                continue;
            };
            let Some(here) = stands[at] else {
                continue;
            };
            for (there, cost) in self.beside(at) {
                let on = away + cost;
                let same = stands[there].is_some_and(|level| (level - here).abs() <= SAME_REACH);
                if !same || on > reach || nearest[there].is_some_and(|known| known.away <= on) {
                    continue;
                }
                nearest[there] = Some(Step {
                    away: on,
                    high: step.high,
                });
                open.push(Reverse((on, there)));
            }
        }
        nearest
    }
}
