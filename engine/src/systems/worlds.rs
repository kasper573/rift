//! The worlds a server runs: one for each area and one for each instance of one, stepped
//! together each tick.

use bevy_ecs::prelude::{Entity, World};
use rayon::prelude::*;

use crate::core::content::Content;
use crate::core::time::{UnixMillis, WallClock};
use crate::systems::area::{AreaDef, AreaId};
use crate::systems::area_instance::AreaInstanceId;
use crate::systems::player::errand::{self, Errand, ErrandOutcome};
use crate::systems::player::roster::{self, Enrollment, ServerRoster};
use crate::systems::player::{ClientId, Departure, Players};
use crate::systems::save::{self, Checkpoints};
use crate::systems::society::{self, ServerSociety, SocietyAsk};
use crate::systems::text::LineText;
use crate::systems::{player, stat, step_area};

/// Which of a server's worlds: the one everyone in `area` shares, or an instance of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorldName {
    pub area: AreaId,
    pub instance: Option<AreaInstanceId>,
}

impl WorldName {
    /// The world everyone in `area` shares.
    pub fn shared(area: AreaId) -> WorldName {
        WorldName {
            area,
            instance: None,
        }
    }
}

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

    /// What tells `name` from every other world the server has run since it started: an area's
    /// place in the list, and past the list an instance's.
    pub fn ordinal(&self, name: WorldName) -> u64 {
        match name.instance {
            Some(instance) => self.areas.len() as u64 + u64::from(instance.0),
            None => self
                .areas
                .iter()
                .position(|&running| running == name.area)
                .expect("every area has a world") as u64,
        }
    }
}

/// What a server keeps above its worlds: who plays where, society, and what saving has under
/// way.
pub struct AboveWorlds {
    pub roster: ServerRoster,
    pub society: ServerSociety,
    pub checkpoints: Checkpoints,
}

impl AboveWorlds {
    /// With a society that holds nothing yet, saving nothing.
    pub fn unsaved() -> AboveWorlds {
        AboveWorlds {
            roster: ServerRoster::default(),
            society: ServerSociety::default(),
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
/// on the server, every errand a world asked be done to a player in another, what their players
/// asked of society, and the checkpoint that saves them all as they stand at this one instant.
/// Each errand is answered, whatever became of its player. Says which characters left their world
/// since the worlds last conferred.
pub fn confer(
    views: &mut [Option<&mut World>],
    above: &mut AboveWorlds,
    now: UnixMillis,
) -> Vec<Departure> {
    let heard = hear(views, above);
    for (asking, errand) in heard.errands {
        let outcome = carry(views, errand.to, errand.deed);
        if let Some(world) = views[asking].as_deref_mut() {
            (errand.then)(world, outcome);
        }
    }
    society::settle(views, above, heard.asked, now);
    save::checkpoint(views, &mut above.checkpoints, heard.gone, now)
}

/// What the worlds had to say since they last conferred.
#[derive(Default)]
struct Heard {
    /// The errands each world asked for, with the world that asked.
    errands: Vec<(usize, Errand)>,
    /// What each world's players asked of society.
    asked: Vec<(ClientId, SocietyAsk)>,
    /// The characters that left a world.
    gone: Vec<Departure>,
}

/// Hears every world out, each once and on a core of its own, and lists everyone on the roster.
/// This is everything a conference asks of a world that nothing happened in, so a world is
/// visited once and what follows costs only what did happen.
fn hear(views: &mut [Option<&mut World>], above: &AboveWorlds) -> Heard {
    let (roster, society) = (&above.roster, &above.society);
    let saving = above.checkpoints.saving();
    // A world lets its last enrollment go as it enrolls anew, which frees it there and then only
    // once the roster no longer lists it: the roster is emptied first, or one core would free
    // every world's.
    roster.list(Vec::new());
    let said: Vec<(Enrollment, Heard)> = views
        .par_iter_mut()
        .enumerate()
        .filter_map(|(asking, world)| Some((asking, world.as_deref_mut()?)))
        .map(|(asking, world)| {
            let errands = errand::take(world).into_iter();
            let heard = Heard {
                errands: errands.map(|errand| (asking, errand)).collect(),
                asked: society::take_asks(world, society),
                gone: save::hear(world, saving),
            };
            (roster::enroll(world, roster), heard)
        })
        .collect();
    let mut all = Heard::default();
    let mut enrolled = Vec::with_capacity(said.len());
    for (enrollment, mut heard) in said {
        enrolled.push(enrollment);
        all.errands.append(&mut heard.errands);
        all.asked.append(&mut heard.asked);
        all.gone.append(&mut heard.gone);
    }
    roster.list(enrolled);
    all
}

fn carry(
    views: &mut [Option<&mut World>],
    to: ClientId,
    deed: impl FnOnce(&mut World, Entity) -> Result<(), LineText>,
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
