//! The worlds a server runs: one for each area, in one list, stepped together each tick.

use bevy_ecs::prelude::{Entity, World};
use rayon::prelude::*;

use crate::core::content::Content;
use crate::core::time::{UnixMillis, WallClock};
use crate::systems::area::{AreaDef, AreaId};
use crate::systems::player::errand::{self, Errand, ErrandOutcome};
use crate::systems::player::roster::{self, Enrollment, ServerRoster};
use crate::systems::player::{ClientId, Departure, Players};
use crate::systems::save::{self, Checkpoints};
use crate::systems::society::Society;
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

/// What a server keeps above its worlds: who plays where, society, and what saving has under
/// way.
pub struct AboveWorlds {
    pub roster: ServerRoster,
    pub society: Society,
    pub checkpoints: Checkpoints,
}

impl AboveWorlds {
    /// With a society that holds nothing yet, saving nothing.
    pub fn unsaved() -> AboveWorlds {
        AboveWorlds {
            roster: ServerRoster::default(),
            society: Society::default(),
            checkpoints: Checkpoints::none(),
        }
    }
}

/// One tick of the worlds anything of a player is in, each on its own, across all cores, once
/// they have conferred: the characters that left a world since the last tick. A world with no
/// client connected and no character in it stands still, untouched, until someone comes.
pub fn tick(
    views: &mut [Option<&mut World>],
    clock: WallClock,
    above: &mut AboveWorlds,
) -> Vec<Departure> {
    let gone = confer(views, above, clock.now);
    views
        .par_iter_mut()
        .filter_map(|world| world.as_deref_mut())
        .filter(|world| player::any_present(world))
        .for_each(|world| step_area(world, clock));
    gone
}

/// What the worlds settle between ticks, the only time one hand holds them all: who plays where
/// on the server, every errand a world asked be done to a player in another, and the checkpoint
/// that saves them all as they stand at this one instant. Each errand is answered, whatever
/// became of its player. Says which characters left their world since the worlds last conferred.
pub fn confer(
    views: &mut [Option<&mut World>],
    above: &mut AboveWorlds,
    now: UnixMillis,
) -> Vec<Departure> {
    for (asking, errand) in enroll(views, &above.roster) {
        let outcome = carry(views, errand.to, errand.deed);
        if let Some(world) = views[asking].as_deref_mut() {
            (errand.then)(world, outcome);
        }
    }
    save::checkpoint(views, &mut above.checkpoints, now)
}

/// Lists everyone on `roster`, each world enrolling its own players on a core of its own, and
/// takes the errands each world asked for since, with the world that asked.
fn enroll(views: &mut [Option<&mut World>], roster: &ServerRoster) -> Vec<(usize, Errand)> {
    // A world lets its last enrollment go as it enrolls anew, which frees it there and then only
    // once the roster no longer lists it: the roster is emptied first, or one core would free
    // every world's.
    roster.list(Vec::new());
    let settled: Vec<(Enrollment, Vec<(usize, Errand)>)> = views
        .par_iter_mut()
        .enumerate()
        .filter_map(|(asking, world)| Some((asking, world.as_deref_mut()?)))
        .map(|(asking, world)| {
            let asked = errand::take(world)
                .into_iter()
                .map(|errand| (asking, errand))
                .collect();
            (roster::enroll(world, roster), asked)
        })
        .collect();
    let (enrolled, asked): (Vec<Enrollment>, Vec<Vec<(usize, Errand)>>) =
        settled.into_iter().unzip();
    roster.list(enrolled);
    asked.into_iter().flatten().collect()
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
