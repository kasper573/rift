use game::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use game::systems::area::transition::{self, Crossing};
use game::systems::memory::{Forget, Memory, Remember, Remembers};
use game::systems::movement::position;
use game::systems::rule::{Encounter, Requirement, Terms};

use crate::support::{Sim, row};

const HOUR: u64 = 3_600_000;

fn at(millis: u64) -> WallClock {
    WallClock {
        now: UnixMillis(millis),
        reset: UtcHour::try_from(4).expect("hour"),
    }
}

fn later(clock: WallClock, seconds: f32) -> WallClock {
    WallClock {
        now: clock.now.after(Seconds(seconds)),
        ..clock
    }
}

#[test]
fn flags_are_remembered_until_forgotten() {
    let clock = at(100 * HOUR);
    let mut memory = Memory::default();
    assert_eq!(memory.recall(row("BribedIlsa"), clock), None);
    memory.remember(row("BribedIlsa"), clock);
    assert_eq!(memory.recall(row("BribedIlsa"), later(clock, 1e7)), Some(1));
    memory.forget(row("BribedIlsa"));
    assert_eq!(memory.recall(row("BribedIlsa"), clock), None);
}

#[test]
fn counters_step_at_most_once_per_cooldown() {
    let start = at(100 * HOUR);
    let mut memory = Memory::default();
    memory.remember(row("TobbVisits"), start);
    memory.remember(row("TobbVisits"), later(start, 60.0));
    memory.remember(row("TobbVisits"), later(start, 599.0));
    assert_eq!(
        memory.recall(row("TobbVisits"), later(start, 599.0)),
        Some(1)
    );
    memory.remember(row("TobbVisits"), later(start, 600.0));
    assert_eq!(
        memory.recall(row("TobbVisits"), later(start, 600.0)),
        Some(2)
    );
}

#[test]
fn timers_run_out() {
    let start = at(100 * HOUR);
    let mut memory = Memory::default();
    memory.remember(row("PellGrudge"), start);
    assert_eq!(
        memory.recall(row("PellGrudge"), later(start, 1_799.0)),
        Some(1)
    );
    assert_eq!(
        memory.recall(row("PellGrudge"), later(start, 1_800.0)),
        None
    );
}

#[test]
fn daily_memories_end_at_the_reset_hour() {
    let before_reset = at(100 * 24 * HOUR + 3 * HOUR);
    let mut memory = Memory::default();
    memory.remember(row("TideChestLooted"), before_reset);
    assert_eq!(
        memory.recall(row("TideChestLooted"), later(before_reset, 3_599.0)),
        Some(1)
    );
    assert_eq!(
        memory.recall(row("TideChestLooted"), later(before_reset, 3_600.0)),
        None
    );
}

#[test]
fn leaving_an_area_forgets_what_only_lasts_there() {
    let mut sim = Sim::area(game::data::area::SPAWN_ID);
    let player = sim.join(1);
    let clock = *sim.world().resource::<WallClock>();
    let dest = position(sim.world(), player).expect("position");
    let mut memory = Memory::default();
    memory.remember(row("PellFighting"), clock);
    memory.remember(row("PellGrudge"), clock);
    sim.world().entity_mut(player).insert((
        memory,
        Crossing {
            dest_area: row("Forest"),
            dest,
        },
    ));

    let travelers = transition::departing(sim.world());

    let carried = &travelers[0].state.memory;
    assert_eq!(carried.recall(row("PellFighting"), clock), None);
    assert_eq!(carried.recall(row("PellGrudge"), clock), Some(1));
}

#[test]
fn choices_write_memory_and_requirements_read_it() {
    let side = Remember(row("SidedWithOrcs"));
    let unside = Forget(row("SidedWithOrcs"));
    let mut sim = Sim::area(game::data::area::SPAWN_ID);
    let player = sim.join(1);
    let sided = Remembers(row("SidedWithOrcs"));

    assert!(!sided.met(sim.world(), player));
    Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&side],
    }
    .settle(sim.world(), player, Encounter::default())
    .expect("settles");
    assert!(sided.met(sim.world(), player));
    Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&unside],
    }
    .settle(sim.world(), player, Encounter::default())
    .expect("settles");
    assert!(!sided.met(sim.world(), player));
}
