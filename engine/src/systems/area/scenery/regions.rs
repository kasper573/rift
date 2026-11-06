//! The quadtree over an area's ground that the scenery stands its covers by, cut finer
//! the nearer the eye. What lies in a leaf of the cut stands piece by piece, as near the eye it
//! must; what lies in a coarser region stands merged, no finer than the view shows the region's
//! nearest edge, so a view over a whole area stands a few meshes, however far it stretches.

use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use super::plan::{Detail, Seen};
use super::{PLACEMENT_CELL, SPLIT_WITHIN, distance_to};
use crate::core::ground::Heightfield;
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{MeterSize, Meters};
use crate::systems::area::Area;

/// The quadtree over an area's ground: the ground its root covers from its corner, and how many
/// levels lie between its root and its leaves.
pub(super) struct Regions {
    ground: Rect<Meters>,
    levels: u32,
}

/// A region of the quadtree: its level (0 the leaves) and its place among the regions of that
/// level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Region {
    pub level: u32,
    pub column: u32,
    pub row: u32,
}

/// A region whose scenery stands merged, and the detail the view shows its nearest edge at, so
/// what stands for it is made again only as that detail changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Merge {
    pub region: Region,
    pub detail: Detail,
}

/// The quadtree cut for a view: the leaves whose scenery stands piece by piece, and the coarser
/// regions whose scenery stands merged, every one within the view's reach.
pub(super) struct Cut {
    pub leaves: HashSet<Region>,
    pub merges: Vec<Merge>,
}

impl Regions {
    pub(super) fn of(area: &Area) -> Regions {
        let ground = area.size.bounds();
        let leaves = (ground.size.width.max(ground.size.height) / PLACEMENT_CELL.0).ceil() as u32;
        Regions {
            ground,
            levels: leaves.max(1).next_power_of_two().trailing_zeros(),
        }
    }

    /// Every region is cut in four while an eye may lie within `SPLIT_WITHIN` of its sides from it
    /// and it is wider than a leaf.
    pub(super) fn cut(&self, heights: &Heightfield, seen: &Seen) -> Cut {
        let ground = heights.range();
        let mut cut = Cut {
            leaves: HashSet::default(),
            merges: Vec::new(),
        };
        let mut open = vec![Region {
            level: self.levels,
            column: 0,
            row: 0,
        }];
        while let Some(region) = open.pop() {
            let bounds = self.bounds(region);
            if !bounds.intersects(&self.ground) || distance_to(bounds, seen.at()) > seen.reach.0 {
                continue;
            }
            let side = bounds.size.width;
            let from_eye = seen.over(bounds, ground.lowest, ground.highest);
            if from_eye >= side * SPLIT_WITHIN {
                cut.merges.push(Merge {
                    region,
                    detail: seen.detail(from_eye),
                });
            } else if region.level == 0 {
                cut.leaves.insert(region);
            } else {
                for (dc, dr) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    open.push(Region {
                        level: region.level - 1,
                        column: region.column * 2 + dc,
                        row: region.row * 2 + dr,
                    });
                }
            }
        }
        cut
    }

    pub(super) fn bounds(&self, region: Region) -> Rect<Meters> {
        let side = PLACEMENT_CELL.0 * (1u32 << region.level) as f32;
        Rect::new(
            Pos::new(
                self.ground.origin.x + region.column as f32 * side,
                self.ground.origin.y + region.row as f32 * side,
            ),
            Size::new(side, side),
        )
    }
}

/// Where the scenery doesn't yet stand all a view takes in. What stood there for an earlier view
/// stands on until it does, so the view never shows a hole, while what the view left elsewhere goes
/// at once rather than standing until the whole view does.
#[derive(Default)]
pub(super) struct Unstood(Vec<Rect<Meters>>);

impl Unstood {
    pub(super) fn add(&mut self, bounds: Rect<Meters>) {
        self.0.push(bounds);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the scenery stands all the view takes in over `bounds`.
    pub(super) fn stands_over(&self, bounds: Rect<Meters>) -> bool {
        !self.0.iter().any(|unstood| unstood.intersects(&bounds))
    }
}

/// Whether `at` lies within `bounds`, its far edges left to the regions beyond, so a point lies in
/// one region of a level alone.
pub(super) fn holds(bounds: Rect<Meters>, at: Pos<Meters>) -> bool {
    at.x >= bounds.min().x
        && at.x < bounds.max().x
        && at.y >= bounds.min().y
        && at.y < bounds.max().y
}
