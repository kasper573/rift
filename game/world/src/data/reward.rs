use crate::data::item::Id as ItemId;
use crate::data::npc::Id as NpcId;
use crate::systems::rewards::{Grant, RewardDef};

crate::table! {
    OrcXp: RewardDef {
        npc: NpcId::Orc,
        amount: 12,
        grant: Grant::Xp,
    },
    OrcHealthPotion: RewardDef {
        npc: NpcId::Orc,
        amount: 1,
        grant: Grant::Item { item: ItemId::HealthPotion, chance: None },
    },
    OrcOrcTusk: RewardDef {
        npc: NpcId::Orc,
        amount: 1,
        grant: Grant::Item { item: ItemId::OrcTusk, chance: Some(50.0) },
    },
    OrcChiefXp: RewardDef {
        npc: NpcId::OrcChief,
        amount: 40,
        grant: Grant::Xp,
    },
    OrcChiefGreaterHealthPotion: RewardDef {
        npc: NpcId::OrcChief,
        amount: 1,
        grant: Grant::Item { item: ItemId::GreaterHealthPotion, chance: Some(75.0) },
    },
    OrcChiefOrcTusk: RewardDef {
        npc: NpcId::OrcChief,
        amount: 2,
        grant: Grant::Item { item: ItemId::OrcTusk, chance: None },
    },
    OrcChiefTribalHelmet: RewardDef {
        npc: NpcId::OrcChief,
        amount: 1,
        grant: Grant::Item { item: ItemId::TribalHelmet, chance: Some(10.0) },
    },
    SkeletonXp: RewardDef {
        npc: NpcId::Skeleton,
        amount: 10,
        grant: Grant::Xp,
    },
    SkeletonBone: RewardDef {
        npc: NpcId::Skeleton,
        amount: 2,
        grant: Grant::Item { item: ItemId::Bone, chance: Some(80.0) },
    },
    SkeletonRustySword: RewardDef {
        npc: NpcId::Skeleton,
        amount: 1,
        grant: Grant::Item { item: ItemId::RustySword, chance: Some(15.0) },
    },
    SkeletonBoneShield: RewardDef {
        npc: NpcId::Skeleton,
        amount: 1,
        grant: Grant::Item { item: ItemId::BoneShield, chance: Some(2.5) },
    },
    BatXp: RewardDef {
        npc: NpcId::Bat,
        amount: 4,
        grant: Grant::Xp,
    },
    BatBatWing: RewardDef {
        npc: NpcId::Bat,
        amount: 1,
        grant: Grant::Item { item: ItemId::BatWing, chance: Some(65.0) },
    },
    VampireBatXp: RewardDef {
        npc: NpcId::VampireBat,
        amount: 8,
        grant: Grant::Xp,
    },
    VampireBatBatWing: RewardDef {
        npc: NpcId::VampireBat,
        amount: 2,
        grant: Grant::Item { item: ItemId::BatWing, chance: Some(65.0) },
    },
    VampireBatHealthPotion: RewardDef {
        npc: NpcId::VampireBat,
        amount: 1,
        grant: Grant::Item { item: ItemId::HealthPotion, chance: Some(25.0) },
    },
}
