//! Stats are per-stat components; the effective value combat reads is the base plus every active
//! effect's delta. These assert the contracts: equipped effects sum onto base, job level counts
//! reached thresholds, equip requirements gate on job/level/stat, and an npc's chase doubles its
//! move speed and reverts once it drops its target.

use std::collections::BTreeMap;

use bevy_ecs::world::World;
use world::core::table::Id;
use world::systems::combat::AttackTarget;
use world::systems::equipment::requirements::{self, RequirementKind};
use world::systems::equipment::{EquipSlot, Equipment};
use world::systems::items::ItemDef;
use world::systems::job::{self, Job};
use world::systems::npc::Npc;
use world::systems::player::{self, Xp};
use world::systems::server_app;
use world::systems::stat::{self, DamageStat, MaxHealthStat, MovementSpeedStat, StatSet};

fn item(name: &str) -> Id<ItemDef> {
    Id::<ItemDef>::by_name(name).expect("a known item")
}

fn equipped(items: &[(EquipSlot, &str)]) -> Equipment {
    Equipment {
        slots: items
            .iter()
            .map(|(slot, name)| (*slot, item(name)))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn effective_sums_equipped_effects_onto_base() {
    let mut app = server_app(Id::new(0));
    let world = app.world_mut();
    let player = world
        .spawn(equipped(&[
            (EquipSlot::Weapon, "rusty_sword"),  // +3 damage
            (EquipSlot::Offhand, "bone_shield"), // +5 max health
        ]))
        .id();
    player::player_stats().apply(world, player);

    assert_eq!(stat::effective(world, player, DamageStat.into()), 9.0); // base 6 + 3
    assert_eq!(stat::effective(world, player, MaxHealthStat.into()), 35.0); // base 30 + 5
    assert_eq!(
        stat::effective(world, player, MovementSpeedStat.into()),
        4.0
    ); // unaffected
}

#[test]
fn job_level_counts_reached_thresholds() {
    for (xp, level) in [
        (0, 1),
        (29, 1),
        (30, 2),
        (89, 2),
        (90, 3),
        (200, 4),
        (9999, 4),
    ] {
        let mut world = World::new();
        let entity = world
            .spawn((
                Job {
                    def: job::default_job(),
                },
                Xp { amount: xp },
            ))
            .id();
        assert_eq!(job::level(&world, entity), level, "xp {xp}");
    }
}

#[test]
fn requirements_gate_on_job_level_and_stat() {
    let mut app = server_app(Id::new(0));
    let world = app.world_mut();
    let player = world
        .spawn((
            Job {
                def: job::default_job(),
            },
            Xp { amount: 30 },                               // level 2
            equipped(&[(EquipSlot::Weapon, "rusty_sword")]), // damage 9
        ))
        .id();
    player::player_stats().apply(world, player);

    let level = |level| RequirementKind::Level(requirements::Level { level });
    let damage_at_least = |min| {
        RequirementKind::Stat(requirements::Stat {
            stat: DamageStat.into(),
            min,
        })
    };
    let job = RequirementKind::Job(requirements::Job {
        job: job::default_job(),
    });

    assert!(requirements::met(world, player, &[level(2)]));
    assert!(!requirements::met(world, player, &[level(3)]));
    assert!(requirements::met(world, player, &[damage_at_least(9.0)]));
    assert!(!requirements::met(world, player, &[damage_at_least(10.0)]));
    assert!(requirements::met(world, player, &[job]));
}

#[test]
fn chasing_npc_doubles_move_speed_then_reverts() {
    let mut app = server_app(Id::new(0));
    let world = app.world_mut();
    let target = world.spawn_empty().id();
    let npc = world
        .spawn(Npc {
            def: Id::new(0),
            group: 0,
        })
        .id();
    let mut base = StatSet::default();
    base.add(MovementSpeedStat.into(), 3.0);
    base.apply(world, npc);

    let speed = |world: &World| stat::effective(world, npc, MovementSpeedStat.into());
    assert_eq!(speed(world), 3.0);

    world.entity_mut(npc).insert(AttackTarget { target });
    assert_eq!(speed(world), 6.0);

    world.entity_mut(npc).remove::<AttackTarget>();
    assert_eq!(speed(world), 3.0);
}
