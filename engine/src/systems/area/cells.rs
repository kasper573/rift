//! How an area's ground is cut into the cells its server runs it as: squares of the game's cell
//! side from the top-left corner, the last column and row taking what's left, each run as a world
//! of its own. An area whose sides fit in one cell is one world.

use crate::core::math::{Pos, Rect, Size};
use crate::core::units::Meters;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cells {
    size: Size<Meters>,
    side: Meters,
    columns: u32,
    rows: u32,
}

/// One cell of an area, by its place in the grid, row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cell(pub u32);

impl Cells {
    pub fn over(size: Size<Meters>, side: Meters) -> Cells {
        Cells {
            size,
            side,
            columns: side_cells(size.width, side),
            rows: side_cells(size.height, side),
        }
    }

    pub fn count(&self) -> usize {
        (self.columns * self.rows) as usize
    }

    pub fn iter(&self) -> impl Iterator<Item = Cell> + use<> {
        (0..self.columns * self.rows).map(Cell)
    }

    /// The cell holding `at`; a point off the ground falls in the nearest cell.
    pub fn at(&self, at: Pos<Meters>) -> Cell {
        let column = ((at.x / self.side.0).floor().max(0.0) as u32).min(self.columns - 1);
        let row = ((at.y / self.side.0).floor().max(0.0) as u32).min(self.rows - 1);
        Cell(row * self.columns + column)
    }

    /// The cells within `reach` of `at`: those whose ground, grown by `reach` on every side, holds it.
    pub fn within(&self, at: Pos<Meters>, reach: Meters) -> impl Iterator<Item = Cell> + use<> {
        let (columns, side) = (self.columns, self.side.0);
        let span = move |low: f32, high: f32, count: u32| {
            let first = ((low / side).floor().max(0.0) as u32).min(count - 1);
            let last = ((high / side).floor().max(0.0) as u32).min(count - 1);
            first..=last
        };
        span(at.y - reach.0, at.y + reach.0, self.rows).flat_map(move |row| {
            span(at.x - reach.0, at.x + reach.0, columns)
                .map(move |column| Cell(row * columns + column))
        })
    }

    pub fn rect(&self, cell: Cell) -> Rect<Meters> {
        let (column, row) = (cell.0 % self.columns, cell.0 / self.columns);
        let origin = Pos::new(column as f32 * self.side.0, row as f32 * self.side.0);
        let far: Pos<Meters> = Pos::new(
            self.size.width.min(origin.x + self.side.0),
            self.size.height.min(origin.y + self.side.0),
        );
        Rect::new(origin, Size::new(far.x - origin.x, far.y - origin.y))
    }
}

fn side_cells(length: f32, side: Meters) -> u32 {
    ((length / side.0).ceil() as u32).max(1)
}
