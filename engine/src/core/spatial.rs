//! Spots on the ground filed by the square cell each stands in, so whatever asks what stands near a
//! point passes over only the cells around it, however much stands elsewhere.

use std::ops::RangeInclusive;

use crate::core::math::{Offset, Pos};
use crate::core::units::Meters;

pub struct SpatialIndex {
    cell: Meters,
    /// Each spot's index, ordered by its cell, row by row.
    filed: Vec<(Cell, u32)>,
    /// The box around every filed spot, so a query reaching none of them ends at once.
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

    /// Files `spots` anew, each by its place among them.
    pub fn refile(&mut self, spots: impl IntoIterator<Item = Pos<Meters>>) {
        self.filed.clear();
        self.bounds = None;
        let cell = self.cell;
        for (index, at) in spots.into_iter().enumerate() {
            self.filed.push((cell_of(cell, at), index as u32));
            self.bounds = Some(match self.bounds {
                None => (at, at),
                Some((min, max)) => (
                    Pos::new(min.x.min(at.x), min.y.min(at.y)),
                    Pos::new(max.x.max(at.x), max.y.max(at.y)),
                ),
            });
        }
        self.filed.sort_unstable();
    }

    /// The indices of the spots that may lie within `within` of `from`, in no particular order:
    /// every one that does, and some a little farther.
    pub fn near(&self, from: Pos<Meters>, within: Meters) -> impl Iterator<Item = usize> + '_ {
        let reach = Offset::new(within.0, within.0);
        let (low, high) = (
            cell_of(self.cell, from - reach),
            cell_of(self.cell, from + reach),
        );
        let rows = match self.bounds {
            Some((min, max))
                if from.x + within.0 >= min.x
                    && from.x - within.0 <= max.x
                    && from.y + within.0 >= min.y
                    && from.y - within.0 <= max.y =>
            {
                low.0.max(cell_of(self.cell, min).0)..=high.0.min(cell_of(self.cell, max).0)
            }
            _ => RangeInclusive::new(1, 0),
        };
        rows.flat_map(move |row| {
            let start = self.filed.partition_point(|(cell, _)| *cell < (row, low.1));
            let end = self
                .filed
                .partition_point(|(cell, _)| *cell <= (row, high.1));
            &self.filed[start..end]
        })
        .map(|&(_, index)| index as usize)
    }
}

fn cell_of(cell: Meters, at: Pos<Meters>) -> Cell {
    (
        (at.y / cell.0).floor() as i32,
        (at.x / cell.0).floor() as i32,
    )
}
