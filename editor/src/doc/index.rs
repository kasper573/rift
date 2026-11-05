use engine::core::math::{Pos, Rect, Size};
use engine::core::spatial::SpatialIndex;
use engine::core::units::Meters;

use super::NodeId;

/// Nodes found by where they lie on the ground, each by the rectangle it takes there.
pub struct NodeGrid<T> {
    placed: Vec<(NodeId, Rect<Meters>, T)>,
    index: SpatialIndex,
    /// Those too wide to file under every cell they cover, asked of by every query.
    wide: Vec<usize>,
}

impl<T> NodeGrid<T> {
    pub fn new(placed: impl IntoIterator<Item = (NodeId, Rect<Meters>, T)>) -> NodeGrid<T> {
        let placed: Vec<(NodeId, Rect<Meters>, T)> = placed.into_iter().collect();
        let wide: Vec<usize> = placed
            .iter()
            .enumerate()
            .filter(|(_, (_, bounds, _))| is_wide(*bounds))
            .map(|(index, _)| index)
            .collect();
        let mut index = SpatialIndex::new(CELL);
        index.refile(placed.iter().map(|(_, bounds, _)| {
            if is_wide(*bounds) {
                Rect::new(bounds.min(), Size::new(0.0, 0.0))
            } else {
                *bounds
            }
        }));
        NodeGrid {
            placed,
            index,
            wide,
        }
    }

    /// Every node whose rectangle meets `reach`, each once.
    pub fn within(
        &self,
        reach: Rect<Meters>,
    ) -> impl Iterator<Item = &(NodeId, Rect<Meters>, T)> + '_ {
        self.index
            .within(reach)
            .filter(|&index| !is_wide(self.placed[index].1))
            .chain(self.wide.iter().copied())
            .map(|index| &self.placed[index])
            .filter(move |(_, bounds, _)| meets(*bounds, reach))
    }

    /// Every node whose rectangle holds `at`.
    pub fn at(&self, at: Pos<Meters>) -> impl Iterator<Item = &(NodeId, Rect<Meters>, T)> + '_ {
        self.within(Rect::new(at, Size::new(0.0, 0.0)))
            .filter(move |(_, bounds, _)| bounds.contains(at))
    }

    /// Every node, in the order they were given.
    pub fn iter(&self) -> impl Iterator<Item = &(NodeId, Rect<Meters>, T)> + '_ {
        self.placed.iter()
    }
}

/// The side of the cells nodes are filed under.
const CELL: Meters = Meters(32.0);
/// How many cells along either side a node may cover and still be filed under each.
const WIDEST: f32 = 16.0;

/// Whether `a` and `b` share a point, edges included.
fn meets(a: Rect<Meters>, b: Rect<Meters>) -> bool {
    a.min().x <= b.max().x
        && b.min().x <= a.max().x
        && a.min().y <= b.max().y
        && b.min().y <= a.max().y
}

fn is_wide(bounds: Rect<Meters>) -> bool {
    bounds.size.width.max(bounds.size.height) > CELL.0 * WIDEST
}
