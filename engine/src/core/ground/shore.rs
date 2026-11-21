//! How far the water lies from its shore: what a sheet on the water tells its shader of where it
//! laps the dry ground.

use crate::core::units::Meters;

/// How far each point of a lattice `columns` across lies from the waterline: out on the water up
/// to `reach`, and on dry ground as far behind it as it lies, below nothing, down to `back`.
/// `depths` say of each point, row by row, how much water stands over it, in metres, nothing or
/// less on dry ground, as far as the ground stands over the water beside it; the points lie
/// `spacing` apart. The waterline runs between a wet point and a dry one beside it where the
/// water between them, falling evenly, is spent, so it follows the ground and not the lattice.
/// Beyond the points beside it, measured by the two sweeps of an eight-way chamfer: at most 8.3%
/// over the straight-line distance.
pub fn shore_distances(
    depths: &[f32],
    columns: usize,
    spacing: Meters,
    reach: Meters,
    back: Meters,
) -> Vec<Meters> {
    let rows = depths.len().checked_div(columns).unwrap_or(0);
    let costs = [spacing.0, spacing.0 * std::f32::consts::SQRT_2];
    let wet: Vec<bool> = depths.iter().map(|depth| *depth > 0.0).collect();
    let mut distance: Vec<f32> = wet
        .iter()
        .map(|&wet| if wet { reach.0 } else { back.0 })
        .collect();
    let beside = |x: usize, y: usize, dx: i64, dy: i64| {
        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
        (nx >= 0 && ny >= 0 && (nx as usize) < columns && (ny as usize) < rows)
            .then(|| ny as usize * columns + nx as usize)
    };
    for y in 0..rows {
        for x in 0..columns {
            let here = y * columns + x;
            if !wet[here] {
                continue;
            }
            for (dx, dy) in AROUND {
                let Some(there) = beside(x, y, dx, dy).filter(|there| !wet[*there]) else {
                    continue;
                };
                let cost = costs[usize::from(dx != 0 && dy != 0)];
                let out = depths[here] / (depths[here] - depths[there]);
                distance[here] = distance[here].min(out * cost);
                distance[there] = distance[there].min((1.0 - out) * cost);
            }
        }
    }
    let mut sweep = |order: &mut dyn Iterator<Item = (usize, usize)>, sign: i64| {
        for (x, y) in order {
            let here = y * columns + x;
            for (dx, dy) in &AROUND[..4] {
                let near =
                    beside(x, y, dx * sign, dy * sign).filter(|near| wet[*near] == wet[here]);
                if let Some(near) = near {
                    let cost = costs[usize::from(*dx != 0 && *dy != 0)];
                    distance[here] = distance[here].min(distance[near] + cost);
                }
            }
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
        .zip(wet)
        .map(|(distance, wet)| {
            Meters(if wet {
                distance.min(reach.0)
            } else {
                -distance.min(back.0)
            })
        })
        .collect()
}

/// The points round a point: first the four a sweep has passed when it comes to the point, then
/// the four it has yet to.
const AROUND: [(i64, i64); 8] = [
    (-1, 0),
    (0, -1),
    (-1, -1),
    (1, -1),
    (1, 0),
    (0, 1),
    (1, 1),
    (-1, 1),
];
