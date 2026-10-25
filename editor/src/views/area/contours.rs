//! Height contours of an area's terrain: a line wherever the ground crosses a whole metre, found
//! by marching the squares between height samples, kept until the terrain changes.

use std::sync::Arc;

use engine::core::math::Pos;
use engine::core::terrain::Heightfield;
use engine::core::units::Meters;

/// Metres of height between contour lines.
pub const INTERVAL: f32 = 1.0;

/// A contour: the height it runs at and the line segments that make it.
pub struct Contour {
    pub height: Meters,
    pub segments: Vec<[Pos<Meters>; 2]>,
}

/// The contours last traced, with the snapshot they were traced from.
#[derive(Default)]
pub struct ContourCache {
    made: Option<u64>,
    contours: Arc<Vec<Contour>>,
}

impl ContourCache {
    /// The contours of `heights`, traced anew when `made` is not the snapshot they came from.
    pub fn of(&mut self, made: u64, heights: &Heightfield) -> Arc<Vec<Contour>> {
        if self.made != Some(made) {
            self.contours = Arc::new(trace(heights));
            self.made = Some(made);
        }
        self.contours.clone()
    }
}

/// Every whole-metre contour of `heights`.
pub fn trace(heights: &Heightfield) -> Vec<Contour> {
    let (columns, rows) = heights.samples();
    if columns < 2 || rows < 2 {
        return Vec::new();
    }
    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    for row in 0..rows {
        for column in 0..columns {
            let height = heights.sample(column, row).0;
            lowest = lowest.min(height);
            highest = highest.max(height);
        }
    }
    let first = (lowest / INTERVAL).ceil() as i32;
    let last = (highest / INTERVAL).floor() as i32;
    (first..=last)
        .map(|step| {
            let level = step as f32 * INTERVAL;
            Contour {
                height: Meters(level),
                segments: at_level(heights, level),
            }
        })
        .filter(|contour| !contour.segments.is_empty())
        .collect()
}

fn at_level(heights: &Heightfield, level: f32) -> Vec<[Pos<Meters>; 2]> {
    let (columns, rows) = heights.samples();
    let mut segments = Vec::new();
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let corners = [
                (column, row),
                (column + 1, row),
                (column + 1, row + 1),
                (column, row + 1),
            ];
            let values = corners.map(|(c, r)| heights.sample(c, r).0 - level);
            let points = corners.map(|(c, r)| Heightfield::sample_pos(c, r));
            let mut crossings: Vec<Pos<Meters>> = Vec::new();
            for edge in 0..4 {
                let (a, b) = (edge, (edge + 1) % 4);
                if (values[a] >= 0.0) != (values[b] >= 0.0) {
                    let t = values[a] / (values[a] - values[b]);
                    crossings.push(points[a].lerp(points[b], t));
                }
            }
            match crossings.len() {
                2 => segments.push([crossings[0], crossings[1]]),
                4 => {
                    segments.push([crossings[0], crossings[1]]);
                    segments.push([crossings[2], crossings[3]]);
                }
                _ => {}
            }
        }
    }
    segments
}
