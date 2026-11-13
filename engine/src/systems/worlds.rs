//! The worlds a server runs: one for each area, in one list, stepped together each tick.

use bevy_ecs::prelude::{Entity, World};
use rayon::prelude::*;

use crate::core::content::Content;
use crate::core::time::WallClock;
use crate::systems::area::{AreaDef, AreaId};
use crate::systems::player::errand::{self, ErrandOutcome};
use crate::systems::player::roster::{self, ServerRoster};
use crate::systems::player::{ClientId, Players};
use crate::systems::{player, stat, step_area};

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

/// One tick of the worlds a client is connected to, each on its own, across all cores, once they
/// have conferred. A world with no one connected stands still, untouched, until someone comes.
pub fn tick(views: &mut [Option<&mut World>], clock: WallClock) {
    confer(views);
    views
        .par_iter_mut()
        .filter_map(|world| world.as_deref_mut())
        .filter(|world| player::any_connected(world))
        .for_each(|world| step_area(world, clock));
}

/// What the worlds settle between ticks, the only time one hand holds them all: who plays where
/// on the server, for the worlds that keep a roster, and every errand a world asked be done to a
/// player in another. Each errand is answered, whatever became of its player.
pub fn confer(views: &mut [Option<&mut World>]) {
    enroll(views);
    for asking in 0..views.len() {
        let errands = views[asking]
            .as_deref_mut()
            .map(errand::take)
            .unwrap_or_default();
        for errand in errands {
            let outcome = carry(views, errand.to, errand.deed);
            if let Some(world) = views[asking].as_deref_mut() {
                (errand.then)(world, outcome);
            }
        }
    }
}

fn enroll(views: &mut [Option<&mut World>]) {
    let keeps = |world: &&mut World| world.contains_resource::<ServerRoster>();
    if !views.iter().flatten().any(keeps) {
        return;
    }
    let roster = ServerRoster::of(
        views
            .iter()
            .flatten()
            .flat_map(|world| roster::enrolled(world))
            .collect(),
    );
    for world in views.iter_mut().flatten().filter(|world| keeps(world)) {
        world.insert_resource(roster.clone());
    }
}

fn carry(
    views: &mut [Option<&mut World>],
    to: ClientId,
    deed: impl FnOnce(&mut World, Entity) -> Result<(), String>,
) -> ErrandOutcome {
    let found = views.iter_mut().flatten().find_map(|world| {
        let character = *world.get_resource::<Players>()?.0.get(&to)?;
        Some((world, character))
    });
    match found {
        Some((world, character)) if stat::stands(world, character) => {
            match deed(world, character) {
                Ok(()) => ErrandOutcome::Done,
                Err(reason) => ErrandOutcome::Refused(reason),
            }
        }
        _ => ErrandOutcome::Unreached,
    }
}
