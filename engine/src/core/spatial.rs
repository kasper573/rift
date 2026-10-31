//! Extents on the ground filed by the square cells each covers, so whatever asks what lies near a
//! point or within a box passes over only the cells there, however much lies elsewhere.

use std::ops::RangeInclusive;

use crate::core::math::{Offset, Pos, Rect};
use crate::core::units::Meters;

pub struct SpatialIndex {
    cell: Meters,
    /// Each extent's index under every cell it covers, ordered by cell, row by row, with the first
    /// cell it covers, so a query spanning several of its cells takes it once.
    filed: Vec<(Cell, Cell, u32)>,
    /// The box around every filed extent, so a query reaching none of them ends at once.
    bounds: Option<(Pos<Meters>, Pos<Meters>)>,
}

type Cell = (i32, i32);

impl SpatialIndex {
    pub fn new(cell: Meters) -> SpatialIndex {
        SpatialIndex {
            cell,
            filed: Vec::new(),
            bounds: None,
        }
    }

    /// Files `extents` anew, each by its place among them. A spot is an extent of no size.
    pub fn refile(&mut self, extents: impl IntoIterator<Item = Rect<Meters>>) {
        self.filed.clear();
        self.bounds = None;
        let cell = self.cell;
        for (index, extent) in extents.into_iter().enumerate() {
            let (min, max) = (extent.min(), extent.max());
            let (first, last) = (cell_of(cell, min), cell_of(cell, max));
            for row in first.0..=last.0 {
                for column in first.1..=last.1 {
                    self.filed.push(((row, column), first, index as u32));
                }
            }
            self.bounds = Some(match self.bounds {
                None => (min, max),
                Some((low, high)) => (
                    Pos::new(low.x.min(min.x), low.y.min(min.y)),
                    Pos::new(high.x.max(max.x), high.y.max(max.y)),
                ),
            });
        }
        self.filed.sort_unstable();
    }

    /// The indices of the extents that may lie within `within` of `from`, in no particular order:
    /// every one that does, and some a little farther.
    pub fn near(&self, from: Pos<Meters>, within: Meters) -> impl Iterator<Item = usize> + '_ {
        let reach = Offset::new(within.0, within.0);
        self.within(Rect::from_points([from - reach, from + reach]))
    }

    /// The indices of the extents that may reach into `reach`, each once, in no particular order:
    /// every one that does, and some a little outside it.
    pub fn within(&self, reach: Rect<Meters>) -> impl Iterator<Item = usize> + '_ {
        let (low, high, rows) = match self.bounds {
            Some((min, max))
                if reach.max().x >= min.x
                    && reach.min().x <= max.x
                    && reach.max().y >= min.y
                    && reach.min().y <= max.y =>
            {
                let (first, last) = (cell_of(self.cell, min), cell_of(self.cell, max));
                let low = cell_of(self.cell, reach.min());
                let high = cell_of(self.cell, reach.max());
                let low = (low.0.max(first.0), low.1.max(first.1));
                let high = (high.0.min(last.0), high.1.min(last.1));
                (low, high, low.0..=high.0)
            }
            _ => ((0, 0), (0, 0), RangeInclusive::new(1, 0)),
        };
        rows.flat_map(move |row| {
            let start = self
                .filed
                .partition_point(|(cell, ..)| *cell < (row, low.1));
            let end = self
                .filed
                .partition_point(|(cell, ..)| *cell <= (row, high.1));
            &self.filed[start..end]
        })
        .filter(move |(cell, first, _)| *cell == (first.0.max(low.0), first.1.max(low.1)))
        .map(|&(.., index)| index as usize)
    }
}

fn cell_of(cell: Meters, at: Pos<Meters>) -> Cell {
    (
        (at.y / cell.0).floor() as i32,
        (at.x / cell.0).floor() as i32,
    )
}
