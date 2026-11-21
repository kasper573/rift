//! Contours of an area's terrain: a line wherever the ground crosses a whole metre of height, and
//! under water one wherever the bed drops to one, two, four metres of depth and on, each found by
//! marching the squares between samples, kept until the terrain changes.

use std::sync::Arc;

use engine::core::ground::{Heightfield, WaterSurface};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::Area;

/// Metres of height between contour lines.
const INTERVAL: f32 = 1.0;
/// The shallowest depth a line is drawn at; each line after it runs twice as deep.
const FIRST_DEPTH: f32 = 1.0;

/// A contour: the height it runs at, or the depth under water, and the line segments that make
/// it.
pub struct Contour {
    pub height: Meters,
    pub segments: Vec<[Pos<Meters>; 2]>,
}

/// The contours of an area's ground: those of its heights, and those of the depths of its water.
#[derive(Default)]
pub struct Contours {
    pub heights: Vec<Contour>,
    pub depths: Vec<Contour>,
}

/// The contours last traced, with the snapshot they were traced from.
#[derive(Default)]
pub struct ContourCache {
    traced: Option<Arc<Area>>,
    contours: Arc<Contours>,
}

impl ContourCache {
    /// The contours of `area`'s ground, traced anew when they were traced for another area.
    pub fn of(&mut self, area: &Arc<Area>) -> Arc<Contours> {
        if !self
            .traced
            .as_ref()
            .is_some_and(|traced| Arc::ptr_eq(traced, area))
        {
            self.contours = Arc::new(Contours {
                heights: trace(&area.heights),
                depths: trace_depths(&area.heights, &area.water),
            });
            self.traced = Some(area.clone());
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
                segments: at_level(heights.samples(), level, |column, row| {
                    heights.sample(column, row).0
                }),
            }
        })
        .filter(|contour| !contour.segments.is_empty())
        .collect()
}

/// A contour wherever `water` stands a metre deep over `heights`, and at every doubling of that
/// down to its deepest.
pub fn trace_depths(heights: &Heightfield, water: &WaterSurface) -> Vec<Contour> {
    let (columns, rows) = heights.samples();
    if columns < 2 || rows < 2 {
        return Vec::new();
    }
    let depth = |column: u32, row: u32| water.sample(column, row).0 - heights.sample(column, row).0;
    let mut deepest = 0.0_f32;
    for row in 0..rows {
        for column in 0..columns {
            deepest = deepest.max(depth(column, row));
        }
    }
    std::iter::successors(Some(FIRST_DEPTH), |depth| Some(depth * 2.0))
        .take_while(|&level| level <= deepest)
        .map(|level| Contour {
            height: Meters(level),
            segments: at_level((columns, rows), level, depth),
        })
        .filter(|contour| !contour.segments.is_empty())
        .collect()
}

/// The segments along which `value`, sampled over a grid of `samples`, crosses `level`.
fn at_level(
    (columns, rows): (u32, u32),
    level: f32,
    value: impl Fn(u32, u32) -> f32,
) -> Vec<[Pos<Meters>; 2]> {
    let mut segments = Vec::new();
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let corners = [
                (column, row),
                (column + 1, row),
                (column + 1, row + 1),
                (column, row + 1),
            ];
            let values = corners.map(|(c, r)| value(c, r) - level);
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
