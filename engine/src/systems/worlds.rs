//! The worlds a server runs: every cell of every area, in one list, stepped together each tick and
//! then handed what crosses between them.

use bevy_ecs::prelude::World;

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::time::WallClock;
use crate::core::units::Meters;
use crate::systems::area::{self, AreaDef, AreaId, Cell, Cells};
use crate::systems::script::Place;
use crate::systems::{mirror, step_areas};

#[derive(Clone)]
pub struct Worlds {
    areas: Vec<(AreaId, Cells)>,
    /// Where each area's cells start in the list.
    starts: Vec<usize>,
    halo: Meters,
}

impl Worlds {
    /// Every cell of every area of the game.
    pub fn of(content: &Content) -> Worlds {
        Worlds::over(
            content
                .ids::<AreaDef>()
                .map(|id| (id, area::cells(content, id)))
                .collect(),
            mirror::halo(content).0,
        )
    }

    /// `count` copies of `area`, each with every cell of its own, as a bench steps them.
    pub fn copies(content: &Content, area: AreaId, count: usize) -> Worlds {
        let cells = area::cells(content, area);
        Worlds::over(vec![(area, cells); count], mirror::halo(content).0)
    }

    fn over(areas: Vec<(AreaId, Cells)>, halo: Meters) -> Worlds {
        let mut starts = Vec::with_capacity(areas.len());
        let mut next = 0;
        for (_, cells) in &areas {
            starts.push(next);
            next += cells.count();
        }
        Worlds {
            areas,
            starts,
            halo,
        }
    }

    /// How far past its cell's edge every world sees.
    pub fn halo(&self) -> Meters {
        self.halo
    }

    pub fn len(&self) -> usize {
        self.starts
            .last()
            .zip(self.areas.last())
            .map_or(0, |(start, (_, cells))| start + cells.count())
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (AreaId, Cell)> + '_ {
        self.areas
            .iter()
            .flat_map(|&(area, cells)| cells.iter().map(move |cell| (area, cell)))
    }

    /// The world running the cell `place` lies in, of the first copy of its area.
    pub fn world_of(&self, place: Place) -> usize {
        let (_, cells) = self.areas[place.area.index()];
        self.starts[place.area.index()] + cells.at(place.at).0 as usize
    }

    /// Whether any area runs as more than one cell, so some world borders on another.
    pub fn has_borders(&self) -> bool {
        self.areas.iter().any(|(_, cells)| cells.count() > 1)
    }

    /// The worlds of the same area as `world`, other than it, whose halo reaches `at`: where what
    /// `world` owns at `at` is held as a ghost.
    pub fn around(&self, world: usize, at: Pos<Meters>) -> impl Iterator<Item = usize> {
        let reach = self.halo;
        let copy = self.starts.partition_point(|&start| start <= world) - 1;
        let (_, cells) = self.areas[copy];
        let start = self.starts[copy];
        cells
            .within(at, reach)
            .map(move |cell| start + cell.0 as usize)
            .filter(move |&other| other != world)
    }
}

/// One tick of every world, as the server steps them: each on its own, across all cores, then the
/// ghosts of what stands near a border handed to the cells beside it.
pub fn tick(worlds: &Worlds, views: &mut [&mut World], clock: WallClock) {
    step_areas(views, clock);
    mirror::exchange(worlds, views);
}
