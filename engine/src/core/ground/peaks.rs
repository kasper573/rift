//! The highest value over every block of a grid of values: what lets a ray pass over ground it
//! flies above without reading it.

/// The highest of a grid's values over blocks of every size: at level `k` each block is `2^k`
/// steps square, from [`FINEST`] up to one block over the whole grid. A block's highest takes in
/// the values on its far edges too, the first of the next block's, so the cells drawn between
/// samples are covered by the block they start in.
///
/// It only ever answers at least the highest: a value lowered after the blocks are counted leaves
/// them as they were, which a ray only passes into, never over.
#[derive(Clone, Debug)]
pub(super) struct Peaks<T> {
    columns: u32,
    rows: u32,
    levels: Vec<Layer<T>>,
}

impl<T: Copy + Ord> Peaks<T> {
    pub(super) fn of(columns: u32, rows: u32, values: &[T]) -> Peaks<T> {
        let side = 1 << FINEST;
        let mut levels = vec![Layer::from_fn(
            blocks(columns, side),
            blocks(rows, side),
            |x, y| {
                highest_of(
                    values,
                    columns,
                    rows,
                    (x * side, y * side),
                    (x * side + side, y * side + side),
                )
            },
        )];
        while let Some(below) = levels
            .last()
            .filter(|layer| layer.columns > 1 || layer.rows > 1)
        {
            let layer =
                Layer::from_fn(below.columns.div_ceil(2), below.rows.div_ceil(2), |x, y| {
                    below.highest((x * 2, y * 2), (x * 2 + 1, y * 2 + 1))
                });
            levels.push(layer);
        }
        Peaks {
            columns,
            rows,
            levels,
        }
    }

    /// At least the highest of the values from `low` to `high`, both included.
    pub(super) fn highest(&self, values: &[T], low: (u32, u32), high: (u32, u32)) -> T {
        let high = (high.0.min(self.columns - 1), high.1.min(self.rows - 1));
        let low = (low.0.min(high.0), low.1.min(high.1));
        let side = 1 << FINEST;
        if high.0 - low.0 <= side && high.1 - low.1 <= side {
            return highest_of(values, self.columns, self.rows, low, high);
        }
        let span = |level: usize, low: u32, high: u32, count: u32| {
            let shift = level as u32 + FINEST;
            let first = (low >> shift).min(count - 1);
            let last = (high.saturating_sub(1) >> shift).clamp(first, count - 1);
            (first, last)
        };
        self.levels
            .iter()
            .enumerate()
            .find_map(|(level, layer)| {
                let (left, right) = span(level, low.0, high.0, layer.columns);
                let (top, bottom) = span(level, low.1, high.1, layer.rows);
                (right - left <= 1 && bottom - top <= 1)
                    .then(|| layer.highest((left, top), (right, bottom)))
            })
            .expect("a top level of one block")
    }

    /// Takes `value` at `(column, row)` into every block holding it.
    pub(super) fn raise(&mut self, column: u32, row: u32, value: T) {
        for (level, layer) in self.levels.iter_mut().enumerate() {
            let shift = level as u32 + FINEST;
            let holding = |at: u32, count: u32| {
                let own = (at >> shift).min(count - 1);
                let before = (at.is_multiple_of(1 << shift) && at > 0).then(|| (at >> shift) - 1);
                [Some(own), before.filter(|&before| before < count)]
            };
            for x in holding(column, layer.columns).into_iter().flatten() {
                for y in holding(row, layer.rows).into_iter().flatten() {
                    let at = (y * layer.columns + x) as usize;
                    layer.highest[at] = layer.highest[at].max(value);
                }
            }
        }
    }
}

/// The level of the smallest blocks counted: smaller ones are read from the values at once.
const FINEST: u32 = 2;

#[derive(Clone, Debug)]
struct Layer<T> {
    columns: u32,
    rows: u32,
    highest: Vec<T>,
}

impl<T: Copy + Ord> Layer<T> {
    fn from_fn(columns: u32, rows: u32, highest: impl Fn(u32, u32) -> T) -> Layer<T> {
        Layer {
            columns,
            rows,
            highest: (0..rows)
                .flat_map(|y| (0..columns).map(move |x| (x, y)))
                .map(|(x, y)| highest(x, y))
                .collect(),
        }
    }

    fn highest(&self, low: (u32, u32), high: (u32, u32)) -> T {
        highest_of(&self.highest, self.columns, self.rows, low, high)
    }
}

/// How many blocks `side` steps wide cover `values` values, each block reaching the first value of
/// the next.
fn blocks(values: u32, side: u32) -> u32 {
    values.saturating_sub(1).div_ceil(side).max(1)
}

/// The highest of the values from `low` to `high`, both included and cut to the grid.
fn highest_of<T: Copy + Ord>(
    values: &[T],
    columns: u32,
    rows: u32,
    low: (u32, u32),
    high: (u32, u32),
) -> T {
    let (right, bottom) = (high.0.min(columns - 1), high.1.min(rows - 1));
    (low.1.min(bottom)..=bottom)
        .flat_map(|y| {
            let start = (y * columns + low.0.min(right)) as usize;
            values[start..=(y * columns + right) as usize]
                .iter()
                .copied()
        })
        .max()
        .expect("a value in every range")
}
