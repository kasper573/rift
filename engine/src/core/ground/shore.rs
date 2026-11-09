//! How far the water lies from its shore: what a sheet on the water tells its shader of where it
//! laps the dry ground.

use crate::core::units::Meters;

/// How far each point of a lattice `columns` across lies from the nearest dry one, up to `reach`,
/// `wet` saying of each point, row by row, whether water stands there, and the points lying
/// `spacing` apart. A dry point lies at nothing, and a point no dry one lies within `reach` of lies
/// at `reach`. Measured by the two sweeps of an eight-way chamfer: at most 8.3% over the
/// straight-line distance.
pub fn shore_distances(
    wet: &[bool],
    columns: usize,
    spacing: Meters,
    reach: Meters,
) -> Vec<Meters> {
    let rows = wet.len().checked_div(columns).unwrap_or(0);
    let mut distance: Vec<f32> = wet
        .iter()
        .map(|&wet| if wet { reach.0 } else { 0.0 })
        .collect();
    let (straight, diagonal) = (spacing.0, spacing.0 * std::f32::consts::SQRT_2);
    let behind = [
        (-1, 0, straight),
        (0, -1, straight),
        (-1, -1, diagonal),
        (1, -1, diagonal),
    ];
    let mut sweep = |order: &mut dyn Iterator<Item = (usize, usize)>, sign: i64| {
        for (x, y) in order {
            let mut best = distance[y * columns + x];
            for &(dx, dy, cost) in &behind {
                let (nx, ny) = (x as i64 + dx * sign, y as i64 + dy * sign);
                if nx >= 0 && ny >= 0 && (nx as usize) < columns && (ny as usize) < rows {
                    best = best.min(distance[ny as usize * columns + nx as usize] + cost);
                }
            }
            distance[y * columns + x] = best;
        }
    };
    sweep(
        &mut (0..rows).flat_map(|y| (0..columns).map(move |x| (x, y))),
        1,
    );
    sweep(
        &mut (0..rows)
            .rev()
            .flat_map(|y| (0..columns).rev().map(move |x| (x, y))),
        -1,
    );
    distance
        .into_iter()
        .map(|distance| Meters(distance.min(reach.0)))
        .collect()
}
