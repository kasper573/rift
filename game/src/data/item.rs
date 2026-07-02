use crate::core::assets::AssetRef;
use crate::core::time::Seconds;
use crate::data::sfx::Id as SfxId;
use crate::systems::effect::Effect;
use crate::systems::equipment::{EquipmentSlot, Requirement};
use crate::systems::item::{ItemDef, ItemKind, ItemSfx, Stackable};
use crate::systems::stat::{Stat, StatKind};

crate::table! {
    HealthPotion: ItemDef {
        display_name: "Health Potion",
        icon: AssetRef("icons/potion/red_potion.png"),
        sfx: ItemSfx { on_use: Some(SfxId::Heal01), drop: Some(SfxId::Landing01) },
        stackable: Some(Stackable { max: 10 }),
        effects: &[],
        kind: ItemKind::Consumable { health_bonus: 10.0, duration: Seconds(0.0) },
    },
    GreaterHealthPotion: ItemDef {
        display_name: "Greater Health Potion",
        icon: AssetRef("icons/potion/red_potion_3.png"),
        sfx: ItemSfx { on_use: Some(SfxId::Heal01), drop: Some(SfxId::Landing01) },
        stackable: Some(Stackable { max: 10 }),
        effects: &[Effect::StatModifier(Stat { kind: StatKind::Damage, value: 3.0 })],
        kind: ItemKind::Consumable { health_bonus: 25.0, duration: Seconds(30.0) },
    },
    BatWing: ItemDef {
        display_name: "Bat Wing",
        icon: AssetRef("icons/monster_part/feather.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Landing01) },
        stackable: Some(Stackable { max: 50 }),
        effects: &[Effect::StatModifier(Stat { kind: StatKind::MovementSpeed, value: 0.5 })],
        kind: ItemKind::Resource,
    },
    Bone: ItemDef {
        display_name: "Bone",
        icon: AssetRef("icons/monster_part/bone.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Landing01) },
        stackable: Some(Stackable { max: 50 }),
        effects: &[],
        kind: ItemKind::Resource,
    },
    OrcTusk: ItemDef {
        display_name: "Orc Tusk",
        icon: AssetRef("icons/monster_part/skull.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Landing01) },
        stackable: Some(Stackable { max: 50 }),
        effects: &[],
        kind: ItemKind::Resource,
    },
    RustySword: ItemDef {
        display_name: "Rusty Sword",
        icon: AssetRef("icons/weapon_and_tool/iron_sword.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Block01) },
        stackable: None,
        effects: &[Effect::StatModifier(Stat { kind: StatKind::Damage, value: 3.0 })],
        kind: ItemKind::Equipment { slot: EquipmentSlot::Weapon, requirements: &[] },
    },
    BoneShield: ItemDef {
        display_name: "Bone Shield",
        icon: AssetRef("icons/weapon_and_tool/wooden_shield.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Block01) },
        stackable: None,
        effects: &[Effect::StatModifier(Stat { kind: StatKind::MaxHealth, value: 5.0 })],
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Offhand,
            requirements: &[Requirement::Level(2)],
        },
    },
    TribalHelmet: ItemDef {
        display_name: "Tribal Helmet",
        icon: AssetRef("icons/equipment/leather_helmet.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Block01) },
        stackable: None,
        effects: &[
            Effect::StatModifier(Stat { kind: StatKind::MaxHealth, value: 8.0 }),
            Effect::StatModifier(Stat { kind: StatKind::Range, value: 0.2 }),
        ],
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Head,
            requirements: &[Requirement::Level(3)],
        },
    },
    FruloosRock: ItemDef {
        display_name: "Just a rock",
        icon: AssetRef("icons/misc/rune_stone.png"),
        sfx: ItemSfx { on_use: None, drop: Some(SfxId::Block01) },
        stackable: None,
        effects: &[
            Effect::StatModifier(Stat { kind: StatKind::MaxHealth, value: -10.0 }),
        ],
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Head,
            requirements: &[],
        },
    },
}
