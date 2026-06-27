use crate::core::tiling::Tiles;
use crate::data::item::Id as ItemId;
use crate::systems::actor::Rgba;
use crate::systems::npc::{Aggressive, Defensive, NpcDef, Pacifist, Protective};
use crate::systems::rewards::Reward;
use crate::systems::stat::{StatKind, Stats};

crate::table! {
    Orc: NpcDef {
        display_name: "Orc",
        model: "orc",
        tint: Rgba(0xffffffff),
        ai: &Defensive,
        stats: Stats(vec![
            StatKind::Health.of(25.0),
            StatKind::MaxHealth.of(25.0),
            StatKind::Damage.of(3.0),
            StatKind::AttackSpeed.of(1.0),
            StatKind::AttackDelay.of(400.0),
            StatKind::Range.of(1.0),
            StatKind::MovementSpeed.of(1.0),
        ]),
        aggro: Tiles(7.0),
        rewards: &[
            Reward::Xp(12),
            Reward::Item { item: ItemId::HealthPotion, chance: None, amount: 1 },
            Reward::Item { item: ItemId::OrcTusk, chance: Some(50.0), amount: 1 },
        ]
    },
    OrcChief: NpcDef {
        display_name: "Orc Chief",
        model: "orc",
        tint: Rgba(0xffb070ff),
        ai: &Protective,
        stats: Stats(vec![
            StatKind::Health.of(60.0),
            StatKind::MaxHealth.of(60.0),
            StatKind::Damage.of(6.0),
            StatKind::AttackSpeed.of(0.8),
            StatKind::AttackDelay.of(600.0),
            StatKind::Range.of(1.2),
            StatKind::MovementSpeed.of(1.2),
        ]),
        aggro: Tiles(9.0),
        rewards: &[
            Reward::Xp(40),
            Reward::Item { item: ItemId::GreaterHealthPotion, chance: Some(75.0), amount: 1 },
            Reward::Item { item: ItemId::OrcTusk, chance: None, amount: 1 },
            Reward::Item { item: ItemId::TribalHelmet, chance: Some(10.0), amount: 1 },
        ]
    },
    Skeleton: NpcDef {
        display_name: "Skeleton",
        model: "skeleton",
        tint: Rgba(0xffffffff),
        ai: &Aggressive,
        stats: Stats(vec![
            StatKind::Health.of(18.0),
            StatKind::MaxHealth.of(18.0),
            StatKind::Damage.of(3.0),
            StatKind::AttackSpeed.of(1.0),
            StatKind::AttackDelay.of(400.0),
            StatKind::Range.of(1.0),
            StatKind::MovementSpeed.of(1.0),
        ]),
        aggro: Tiles(8.0),
        rewards: &[
            Reward::Xp(10),
            Reward::Item { item: ItemId::Bone, chance: Some(80.0), amount: 2 },
            Reward::Item { item: ItemId::RustySword, chance: Some(15.0), amount: 1 },
            Reward::Item { item: ItemId::BoneShield, chance: Some(2.5), amount: 1 },
        ]
    },
    Bat: NpcDef {
        display_name: "Bat",
        model: "bat",
        tint: Rgba(0xffffffff),
        ai: &Pacifist,
        stats: Stats(vec![
            StatKind::Health.of(8.0),
            StatKind::MaxHealth.of(8.0),
            StatKind::Damage.of(1.0),
            StatKind::AttackSpeed.of(1.5),
            StatKind::AttackDelay.of(300.0),
            StatKind::Range.of(1.0),
            StatKind::MovementSpeed.of(1.25),
        ]),
        aggro: Tiles(5.0),
        rewards: &[
            Reward::Xp(4),
            Reward::Item { item: ItemId::BatWing, chance: Some(65.0), amount: 4 },
        ],
    },
    VampireBat: NpcDef {
        display_name: "Vampire Bat",
        model: "bat",
        tint: Rgba(0xff7788ff),
        ai: &Aggressive,
        stats: Stats(vec![
            StatKind::Health.of(12.0),
            StatKind::MaxHealth.of(12.0),
            StatKind::Damage.of(2.0),
            StatKind::AttackSpeed.of(2.0),
            StatKind::AttackDelay.of(200.0),
            StatKind::Range.of(1.0),
            StatKind::MovementSpeed.of(1.5),
        ]),
        aggro: Tiles(8.0),
        rewards: &[
            Reward::Xp(8),
            Reward::Item { item: ItemId::BatWing, chance: Some(65.0), amount: 2 },
            Reward::Item { item: ItemId::HealthPotion, chance: Some(25.0), amount: 1 },
        ]
    },
}
