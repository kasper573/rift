//! The worlds a server runs: one for each area, in one list, stepped together each tick.

use bevy_ecs::prelude::World;
use rayon::prelude::*;

use crate::core::content::Content;
use crate::core::time::WallClock;
use crate::systems::area::{AreaDef, AreaId};
use crate::systems::{player, step_area};

#[derive(Clone)]
pub struct Worlds {
    areas: Vec<AreaId>,
}

impl Worlds {
    /// A world for every area of the game.
    pub fn of(content: &Content) -> Worlds {
        Worlds {
            areas: content.ids::<AreaDef>().collect(),
        }
    }

    /// `count` worlds of `area`, as a bench steps them.
    pub fn copies(area: AreaId, count: usize) -> Worlds {
        Worlds {
            areas: vec![area; count],
        }
    }

    pub fn len(&self) -> usize {
        self.areas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    /// The area of each world, in the list's order.
    pub fn iter(&self) -> impl Iterator<Item = AreaId> + '_ {
        self.areas.iter().copied()
    }

    /// The first world running `area`.
    pub fn world_of(&self, area: AreaId) -> usize {
        self.areas
            .iter()
            .position(|&running| running == area)
            .expect("every area has a world")
    }
}

/// One tick of the worlds a client is connected to, each on its own, across all cores. A world
/// with no one connected stands still, untouched, until someone comes.
pub fn tick(views: &mut [Option<&mut World>], clock: WallClock) {
    views
        .par_iter_mut()
        .filter_map(|world| world.as_deref_mut())
        .filter(|world| player::any_connected(world))
        .for_each(|world| step_area(world, clock));
}
