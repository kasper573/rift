//! The worlds a server runs: every cell of every area, in one list, stepped together each tick and
//! then handed what crosses between them.

use bevy_ecs::prelude::{Resource, World};
use rayon::prelude::*;

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::time::WallClock;
use crate::core::units::Meters;
use crate::systems::area::{self, AreaDef, AreaId, Cell, Cells};
use crate::systems::handoff::{self, Moved};
use crate::systems::script::Place;
use crate::systems::{mirror, player, step_area};

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

    /// The worlds of the cells beside `world`'s in its area.
    pub fn beside(&self, world: usize) -> impl Iterator<Item = usize> + use<> {
        let copy = self.starts.partition_point(|&start| start <= world) - 1;
        let (_, cells) = self.areas[copy];
        let start = self.starts[copy];
        cells
            .beside(Cell((world - start) as u32))
            .map(move |cell| start + cell.0 as usize)
    }

    /// The worlds a tick may touch on account of a client connected to `world`: its own, those
    /// beside it, which wake, and those beside them, which the actors of a waking world may walk
    /// into or post to.
    pub fn reached_from(&self, world: usize) -> impl Iterator<Item = usize> + use<> {
        let mut reached: Vec<usize> = std::iter::once(world)
            .chain(self.beside(world))
            .flat_map(|near| std::iter::once(near).chain(self.beside(near)))
            .collect();
        reached.sort_unstable();
        reached.dedup();
        reached.into_iter()
    }

    /// Whether any area runs as more than one cell, so some world borders on another.
    pub fn has_borders(&self) -> bool {
        self.areas.iter().any(|(_, cells)| cells.count() > 1)
    }

    /// The world of the cell `at` lies in, when it lies deeper into that cell than half the halo
    /// past `world`'s own: where an actor of `world` standing there changes hands. Half the halo
    /// keeps an actor pacing a border from changing hands back and forth.
    pub fn beyond(&self, world: usize, at: Pos<Meters>) -> Option<usize> {
        let copy = self.starts.partition_point(|&start| start <= world) - 1;
        let (_, cells) = self.areas[copy];
        let start = self.starts[copy];
        let own = cells.rect(Cell((world - start) as u32));
        let past = (own.origin.x - at.x)
            .max(at.x - own.max().x)
            .max(own.origin.y - at.y)
            .max(at.y - own.max().y);
        (past > self.halo.0 / 2.0).then(|| start + cells.at(at).0 as usize)
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

/// One tick of the worlds that are awake, as the server steps them: each on its own, across all
/// cores, then the ghosts of what stands near a border handed to the cells beside it, and what
/// stands deep in another cell handed over to it; the players whose connections must follow
/// their characters. A world sleeps while no client is connected to it or a cell beside it and
/// no mail waits for it: a cell with no one to see it stands still, untouched, until someone
/// comes. A cell whose world isn't made yet sleeps; [`Worlds::reached_from`] says which must be
/// made for a client's.
pub fn tick(worlds: &Worlds, views: &mut [Option<&mut World>], clock: WallClock) -> Vec<Moved> {
    let wake = wake(worlds, views);
    views
        .par_iter_mut()
        .zip(&wake)
        .filter(|(_, wake)| wake.awake())
        .filter_map(|(world, _)| world.as_deref_mut())
        .for_each(|world| step_area(world, clock));
    mirror::exchange(worlds, views, &wake);
    handoff::handoff(worlds, views, &wake)
}

/// How a world stands in a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// Stepped and exchanged with as it was the tick before.
    Awake,
    /// Awake after sleeping: what changed beside it while it slept, it hasn't seen.
    Woken,
    /// Left as it stands.
    Asleep,
}

impl Wake {
    pub fn awake(self) -> bool {
        self != Wake::Asleep
    }
}

/// The worlds to step this tick: those a client is connected to, those beside one, whose actors
/// the players there see as ghosts, and those with mail to land. A world that falls asleep is
/// marked so, so the tick it wakes in knows.
fn wake(worlds: &Worlds, views: &mut [Option<&mut World>]) -> Vec<Wake> {
    let connected: Vec<bool> = views
        .iter()
        .map(|world| world.as_deref().is_some_and(player::any_connected))
        .collect();
    (0..views.len())
        .map(|index| {
            let Some(world) = views[index].as_deref_mut() else {
                return Wake::Asleep;
            };
            let awake = connected[index]
                || mirror::has_mail(world)
                || worlds.beside(index).any(|beside| connected[beside]);
            let slept = world.contains_resource::<Slept>();
            match (awake, slept) {
                (true, false) => Wake::Awake,
                (true, true) => {
                    world.remove_resource::<Slept>();
                    Wake::Woken
                }
                (false, slept) => {
                    if !slept {
                        world.insert_resource(Slept);
                    }
                    Wake::Asleep
                }
            }
        })
        .collect()
}

/// Marks a world that slept through a tick.
#[derive(Resource)]
struct Slept;
