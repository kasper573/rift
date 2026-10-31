//! How an area's ground is cut into the cells its server runs it as: a grid of as many columns
//! and rows as the ground holds of the game's cell size, nearest, evened out over the ground so
//! no cell is a sliver, each run as a world of its own. An area about a cell's size or smaller is
//! one world.

use crate::core::math::{Pos, Rect, Size};
use crate::core::units::Meters;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cells {
    cell: Size<Meters>,
    columns: u32,
    rows: u32,
}

/// One cell of an area, by its place in the grid, row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cell(pub u32);

impl Cells {
    pub fn over(size: Size<Meters>, cell_size: Meters) -> Cells {
        let columns = count_along(size.width, cell_size);
        let rows = count_along(size.height, cell_size);
        Cells {
            cell: Size::new(size.width / columns as f32, size.height / rows as f32),
            columns,
            rows,
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
        let column = ((at.x / self.cell.width).floor().max(0.0) as u32).min(self.columns - 1);
        let row = ((at.y / self.cell.height).floor().max(0.0) as u32).min(self.rows - 1);
        Cell(row * self.columns + column)
    }

    /// The cells within `reach` of `at`: those whose ground, grown by `reach` on every side, holds it.
    pub fn within(&self, at: Pos<Meters>, reach: Meters) -> impl Iterator<Item = Cell> + use<> {
        let (columns, cell) = (self.columns, self.cell);
        let span = move |low: f32, high: f32, step: f32, count: u32| {
            let first = ((low / step).floor().max(0.0) as u32).min(count - 1);
            let last = ((high / step).floor().max(0.0) as u32).min(count - 1);
            first..=last
        };
        span(at.y - reach.0, at.y + reach.0, cell.height, self.rows).flat_map(move |row| {
            span(at.x - reach.0, at.x + reach.0, cell.width, columns)
                .map(move |column| Cell(row * columns + column))
        })
    }

    /// The cells beside `cell`, corners included: those whose ground a halo narrower than a cell
    /// reaches into from it.
    pub fn beside(&self, cell: Cell) -> impl Iterator<Item = Cell> + use<> {
        let (columns, rows) = (self.columns as i64, self.rows as i64);
        let (column, row) = (
            (cell.0 % self.columns) as i64,
            (cell.0 / self.columns) as i64,
        );
        (-1..=1)
            .flat_map(move |dr| (-1..=1).map(move |dc| (row + dr, column + dc)))
            .filter(move |&(r, c)| {
                (r, c) != (row, column) && (0..rows).contains(&r) && (0..columns).contains(&c)
            })
            .map(move |(r, c)| Cell((r * columns + c) as u32))
    }

    pub fn rect(&self, cell: Cell) -> Rect<Meters> {
        let (column, row) = (cell.0 % self.columns, cell.0 / self.columns);
        let origin = Pos::new(
            column as f32 * self.cell.width,
            row as f32 * self.cell.height,
        );
        Rect::new(origin, self.cell)
    }
}

fn count_along(length: f32, cell_size: Meters) -> u32 {
    ((length / cell_size.0).round() as u32).max(1)
}
