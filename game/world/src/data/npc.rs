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
            StatKind::Health.new(25.0),
            StatKind::MaxHealth.new(25.0),
            StatKind::Damage.new(3.0),
            StatKind::AttackSpeed.new(1.0),
            StatKind::AttackDelay.new(400.0),
            StatKind::Range.new(1.0),
            StatKind::MovementSpeed.new(1.0),
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
            StatKind::Health.new(60.0),
            StatKind::MaxHealth.new(60.0),
            StatKind::Damage.new(6.0),
            StatKind::AttackSpeed.new(0.8),
            StatKind::AttackDelay.new(600.0),
            StatKind::Range.new(1.2),
            StatKind::MovementSpeed.new(1.2),
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
            StatKind::Health.new(18.0),
            StatKind::MaxHealth.new(18.0),
            StatKind::Damage.new(3.0),
            StatKind::AttackSpeed.new(1.0),
            StatKind::AttackDelay.new(400.0),
            StatKind::Range.new(1.0),
            StatKind::MovementSpeed.new(1.0),
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
            StatKind::Health.new(8.0),
            StatKind::MaxHealth.new(8.0),
            StatKind::Damage.new(1.0),
            StatKind::AttackSpeed.new(1.5),
            StatKind::AttackDelay.new(300.0),
            StatKind::Range.new(1.0),
            StatKind::MovementSpeed.new(1.25),
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
            StatKind::Health.new(12.0),
            StatKind::MaxHealth.new(12.0),
            StatKind::Damage.new(2.0),
            StatKind::AttackSpeed.new(2.0),
            StatKind::AttackDelay.new(200.0),
            StatKind::Range.new(1.0),
            StatKind::MovementSpeed.new(1.5),
        ]),
        aggro: Tiles(8.0),
        rewards: &[
            Reward::Xp(8),
            Reward::Item { item: ItemId::BatWing, chance: Some(65.0), amount: 2 },
            Reward::Item { item: ItemId::HealthPotion, chance: Some(25.0), amount: 1 },
        ]
    },
}
