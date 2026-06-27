use crate::core::tiling::Tiles;
use crate::systems::actor::Rgba;
use crate::systems::npc::{Aggressive, Defensive, NpcDef, Pacifist, Protective};
use crate::systems::stat::{Stat, Stats};

crate::table! {
    Orc: NpcDef {
        display_name: "Orc",
        model: "orc",
        tint: Rgba(0xffffffff),
        ai: &Defensive,
        stats: Stats(vec![
            Stat::Health(25.0), Stat::MaxHealth(25.0), Stat::Damage(3.0),
            Stat::AttackSpeed(1.0), Stat::AttackDelay(400.0), Stat::Range(1.0),
            Stat::MovementSpeed(1.0),
        ]),
        aggro: Tiles(7.0),
    },
    OrcChief: NpcDef {
        display_name: "Orc Chief",
        model: "orc",
        tint: Rgba(0xffb070ff),
        ai: &Protective,
        stats: Stats(vec![
            Stat::Health(60.0), Stat::MaxHealth(60.0), Stat::Damage(6.0),
            Stat::AttackSpeed(0.8), Stat::AttackDelay(600.0), Stat::Range(1.2),
            Stat::MovementSpeed(1.2),
        ]),
        aggro: Tiles(9.0),
    },
    Skeleton: NpcDef {
        display_name: "Skeleton",
        model: "skeleton",
        tint: Rgba(0xffffffff),
        ai: &Aggressive,
        stats: Stats(vec![
            Stat::Health(18.0), Stat::MaxHealth(18.0), Stat::Damage(3.0),
            Stat::AttackSpeed(1.0), Stat::AttackDelay(400.0), Stat::Range(1.0),
            Stat::MovementSpeed(1.0),
        ]),
        aggro: Tiles(8.0),
    },
    Bat: NpcDef {
        display_name: "Bat",
        model: "bat",
        tint: Rgba(0xffffffff),
        ai: &Pacifist,
        stats: Stats(vec![
            Stat::Health(8.0), Stat::MaxHealth(8.0), Stat::Damage(1.0),
            Stat::AttackSpeed(1.5), Stat::AttackDelay(300.0), Stat::Range(1.0),
            Stat::MovementSpeed(1.25),
        ]),
        aggro: Tiles(5.0),
    },
    VampireBat: NpcDef {
        display_name: "Vampire Bat",
        model: "bat",
        tint: Rgba(0xff7788ff),
        ai: &Aggressive,
        stats: Stats(vec![
            Stat::Health(12.0), Stat::MaxHealth(12.0), Stat::Damage(2.0),
            Stat::AttackSpeed(2.0), Stat::AttackDelay(200.0), Stat::Range(1.0),
            Stat::MovementSpeed(1.5),
        ]),
        aggro: Tiles(8.0),
    },
}
