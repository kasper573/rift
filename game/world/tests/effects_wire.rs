//! Effect commands ride the wire inside `TimedEffects`, which bevy_replicon serializes with postcard
//! — a non-self-describing binary format. So an [`EffectCommand`] (and the other player-only
//! replicated components) must round-trip through postcard, or replicating a buffed/geared actor
//! breaks replication and freezes the world for clients.

use world::core::time::Seconds;
use world::systems::effect::{EffectCommand, TimedEffect, TimedEffects};

fn buff_command() -> EffectCommand {
    world::systems::item::items()
        .iter()
        .find(|item| item.id.as_str() == "greater_health_potion")
        .expect("the buff potion exists")
        .effects
        .first()
        .cloned()
        .expect("its effect command is defined")
}

#[test]
fn effect_command_round_trips_through_postcard() {
    let command = buff_command();
    let bytes = postcard::to_allocvec(&command).expect("serialize EffectCommand");
    let back: EffectCommand = postcard::from_bytes(&bytes).expect("deserialize EffectCommand");
    assert_eq!(command, back);
}

#[test]
fn timed_effects_round_trip_through_postcard() {
    let timed = TimedEffects(vec![TimedEffect {
        command: buff_command(),
        until: Seconds(42.0),
    }]);
    let bytes = postcard::to_allocvec(&timed).expect("serialize TimedEffects");
    let back: TimedEffects = postcard::from_bytes(&bytes).expect("deserialize TimedEffects");
    assert_eq!(timed, back);
}

#[test]
fn equipment_round_trips_through_postcard() {
    use std::collections::BTreeMap;
    use world::core::table::Id;
    use world::systems::equipment::{Equipment, WeaponSlot};
    use world::systems::item::ItemDef;

    let sword = Id::<ItemDef>::by_name("rusty_sword").expect("the sword exists");
    let equipment = Equipment {
        slots: BTreeMap::from([(WeaponSlot.into(), sword)]),
    };
    let bytes = postcard::to_allocvec(&equipment).expect("serialize Equipment");
    let back: Equipment = postcard::from_bytes(&bytes).expect("deserialize Equipment");
    assert_eq!(equipment, back);
}

#[test]
fn job_round_trips_through_postcard() {
    use world::systems::job::{self, Job};

    let job = Job {
        def: job::default_job(),
    };
    let bytes = postcard::to_allocvec(&job).expect("serialize Job");
    let back: Job = postcard::from_bytes(&bytes).expect("deserialize Job");
    assert_eq!(job, back);
}
