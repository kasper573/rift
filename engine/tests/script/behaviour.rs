use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::combat::{AttackRequest, Attitude};
use engine::systems::movement::{self, position};
use engine::systems::npc::body;
use engine::systems::stat::{self, StatKind};
use testkit::Sim;

use crate::game::{field, health, spawn};

const FAR: Pos<Meters> = Pos::new(28.0, 28.0);
/// Long enough for a blow to land on a monster that wanders off as fast as the player follows: the
/// chase ends where the field does, or where the wander turns back.
const CHASE: f32 = 20.0;

/// An aggressive monster picks a character in its aggro range and drops one beyond it; a
/// defensive one fights only whoever attacks it, a protective one whoever attacks its pack, and a
/// pacifist one never fights back.
#[test]
fn a_monster_fights_as_its_attitude_has_it() {
    {
        let (mut sim, player) = field();
        let hunter = monster(&mut sim, "hunter", player, 1);

        assert_eq!(sim.world.get::<Attitude>(hunter), Some(&Attitude::Hostile));
        assert!(targets(&mut sim, hunter, player, 3.0));

        movement::relocate(&mut sim.world, player, FAR);
        assert!(sim.run_until(5.0, |world| body::target(world, hunter).is_none()));
    }
    {
        let (mut sim, player) = field();
        let guard = monster(&mut sim, "guard", player, 1);

        assert!(!targets(&mut sim, guard, player, 3.0));

        attack(&mut sim, guard);
        assert!(targets(&mut sim, guard, player, CHASE));
    }
    {
        let (mut sim, player) = field();
        let member = monster(&mut sim, "bat", player, 4);
        let chief = monster(&mut sim, "chief", player, 4);

        assert!(!targets(&mut sim, chief, player, 3.0));

        // The pack wandered meanwhile, and the chief guards it only within its aggro range.
        let beside = position(&sim.world, player).expect("a position") + Offset::new(1.0, 1.0);
        movement::relocate(&mut sim.world, member, beside);
        movement::relocate(&mut sim.world, chief, beside);
        attack(&mut sim, member);
        assert!(targets(&mut sim, chief, player, 5.0));
    }
    {
        let (mut sim, player) = field();
        let bat = monster(&mut sim, "bat", player, 1);
        let full = health(&sim, bat);

        attack(&mut sim, bat);
        assert!(
            sim.run_until(CHASE, |world| stat::effective(world, bat, StatKind::Health)
                < full)
        );

        assert!(!targets(&mut sim, bat, player, 3.0));
        assert_eq!(sim.world.get::<Attitude>(bat), Some(&Attitude::Hostile));
    }
}

#[test]
fn two_npcs_targeting_each_other_fight() {
    let (mut sim, player) = field();
    let first = spawn(&mut sim, "brawler", player, Offset::new(2.0, 0.0), 1);
    let second = spawn(&mut sim, "brawler", player, Offset::new(0.0, 2.0), 2);
    sim.tick();
    let full = [health(&sim, first), health(&sim, second)];

    assert!(sim.run_until(5.0, |world| {
        body::target(world, first) == Some(second) && body::target(world, second) == Some(first)
    }));
    assert!(sim.run_until(10.0, |world| {
        stat::effective(world, first, StatKind::Health) < full[0]
            && stat::effective(world, second, StatKind::Health) < full[1]
    }));
}

/// A monster of the npc row under `key`, set down three tiles from `near`.
fn monster(sim: &mut Sim, key: &str, near: Entity, pack: u32) -> Entity {
    let monster = spawn(sim, key, near, Offset::new(3.0, 0.0), pack);
    sim.tick();
    monster
}

fn attack(sim: &mut Sim, target: Entity) {
    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target });
}

fn targets(sim: &mut Sim, npc: Entity, player: Entity, within: f32) -> bool {
    sim.run_until(within, |world| body::target(world, npc) == Some(player))
}
