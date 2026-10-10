use crate::core::assets::AssetRef;
use crate::core::time::Seconds;
use crate::data::quest::Id as QuestId;
use crate::data::sfx::Id as SfxId;
use crate::systems::equipment::EquipmentSlot;
use crate::systems::item::{Flavor, ItemDef, ItemKind, ItemModule, ItemSfx, Stack};
use crate::systems::job::MinLevel;
use crate::systems::quest::OfferQuest;
use crate::systems::stat::{Stat, StatKind};

crate::table! {
    Gold: ItemDef {
        name: "Gold",
        icon: AssetRef("icons/misc/golden_coin.png"),
        sfx: ItemSfx { pickup: SfxId::PickupCoins, trade: SfxId::TradeCoins, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Minted somewhere across the sea. Nobody on the island asks where." }),
            ItemModule::Stack(Stack { max: u32::MAX }),
            ItemModule::Currency,
        ],
    },
    BoneToken: ItemDef {
        name: "Bone Token",
        icon: AssetRef("icons/misc/silver_coin.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBone, trade: SfxId::TradeBone, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Wren carves her own coin. Only her stall takes it, and only she knows from what." }),
            ItemModule::Stack(Stack { max: u32::MAX }),
            ItemModule::Currency,
        ],
    },
    HealthPotion: ItemDef {
        name: "Health Potion",
        icon: AssetRef("icons/potion/red_potion.png"),
        sfx: ItemSfx { pickup: SfxId::PickupGlass, trade: SfxId::TradeGlass, drop: SfxId::Landing01, on_use: Some(SfxId::Heal01) },
        kind: ItemKind::Consumable { health_bonus: 10.0, lasts: Seconds(0.0) },
        modules: &[
            ItemModule::Flavor(Flavor { text: "Tastes of copper and seaweed. Works anyway." }),
            ItemModule::Stack(Stack { max: 10 }),
        ],
    },
    GreaterHealthPotion: ItemDef {
        name: "Greater Health Potion",
        icon: AssetRef("icons/potion/red_potion_3.png"),
        sfx: ItemSfx { pickup: SfxId::PickupGlass, trade: SfxId::TradeGlass, drop: SfxId::Landing01, on_use: Some(SfxId::Heal01) },
        kind: ItemKind::Consumable { health_bonus: 25.0, lasts: Seconds(30.0) },
        modules: &[
            ItemModule::Flavor(Flavor { text: "Thick as tar and twice as bitter. For a while after, your blows land harder." }),
            ItemModule::Stack(Stack { max: 10 }),
            ItemModule::Stat(Stat { kind: StatKind::Damage, value: 3.0 }),
        ],
    },
    BatWing: ItemDef {
        name: "Bat Wing",
        icon: AssetRef("icons/monster_part/feather.png"),
        sfx: ItemSfx { pickup: SfxId::PickupFlesh, trade: SfxId::TradeFlesh, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Still twitches on cold nights. Carry a few and you feel oddly light on your feet." }),
            ItemModule::Stack(Stack { max: 50 }),
            ItemModule::Stat(Stat { kind: StatKind::MovementSpeed, value: 0.5 }),
        ],
    },
    Bone: ItemDef {
        name: "Bone",
        icon: AssetRef("icons/monster_part/bone.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBone, trade: SfxId::TradeBone, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Bleached by sun and salt. Wren will want it." }),
            ItemModule::Stack(Stack { max: 50 }),
        ],
    },
    OrcTusk: ItemDef {
        name: "Orc Tusk",
        icon: AssetRef("icons/monster_part/skull.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBone, trade: SfxId::TradeBone, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Yellowed and chipped. Mara pays well for these, and Ugra mourns every one." }),
            ItemModule::Stack(Stack { max: 50 }),
        ],
    },
    RustySword: ItemDef {
        name: "Rusty Sword",
        icon: AssetRef("icons/weapon_and_tool/iron_sword.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBlade, trade: SfxId::TradeBlade, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Equipment { slot: EquipmentSlot::Weapon, requires: &[] },
        modules: &[
            ItemModule::Flavor(Flavor { text: "More rust than sword, but the edge still bites." }),
            ItemModule::Stat(Stat { kind: StatKind::Damage, value: 3.0 }),
        ],
    },
    BoneShield: ItemDef {
        name: "Bone Shield",
        icon: AssetRef("icons/weapon_and_tool/wooden_shield.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBone, trade: SfxId::TradeBone, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Offhand,
            requires: &[&MinLevel(2)],
        },
        modules: &[
            ItemModule::Flavor(Flavor { text: "Ribs lashed to a plank. It creaks when struck, but it holds." }),
            ItemModule::Stat(Stat { kind: StatKind::MaxHealth, value: 5.0 }),
        ],
    },
    TribalHelmet: ItemDef {
        name: "Tribal Helmet",
        icon: AssetRef("icons/equipment/leather_helmet.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBone, trade: SfxId::TradeBone, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Head,
            requires: &[&MinLevel(3)],
        },
        modules: &[
            ItemModule::Flavor(Flavor { text: "The bone helmet of an orc chief. It earns you odd looks on the road." }),
            ItemModule::Stat(Stat { kind: StatKind::MaxHealth, value: 8.0 }),
            ItemModule::Stat(Stat { kind: StatKind::Range, value: 0.2 }),
        ],
    },
    FruloosRock: ItemDef {
        name: "Just a rock",
        icon: AssetRef("icons/misc/rune_stone.png"),
        sfx: ItemSfx { pickup: SfxId::PickupStone, trade: SfxId::TradeStone, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Head,
            requires: &[],
        },
        modules: &[
            ItemModule::Flavor(Flavor { text: "It's a rock. Wearing it on your head was your idea." }),
            ItemModule::Stat(Stat { kind: StatKind::MaxHealth, value: -10.0 }),
        ],
    },
    FishSteak: ItemDef {
        name: "Fish Steak",
        icon: AssetRef("icons/food/fish_steak.png"),
        sfx: ItemSfx { pickup: SfxId::PickupFlesh, trade: SfxId::TradeFlesh, drop: SfxId::Landing01, on_use: Some(SfxId::Heal01) },
        kind: ItemKind::Consumable { health_bonus: 15.0, lasts: Seconds(0.0) },
        modules: &[
            ItemModule::Flavor(Flavor { text: "Grilled over driftwood on Tobb's pier. Even a road warden can be swayed by one." }),
            ItemModule::Stack(Stack { max: 10 }),
        ],
    },
    LuckyLure: ItemDef {
        name: "Lucky Lure",
        icon: AssetRef("icons/ore_and_gem/pearl.png"),
        sfx: ItemSfx { pickup: SfxId::PickupTrinket, trade: SfxId::TradeTrinket, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Tobb swears it has never come back without a fish. Tobb swears a lot of things." }),
        ],
    },
    RoadPass: ItemDef {
        name: "Road Pass",
        icon: AssetRef("icons/misc/scroll.png"),
        sfx: ItemSfx { pickup: SfxId::PickupPaper, trade: SfxId::TradePaper, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Stamped by Ilsa herself. It opens the forest road, and Bram's ferry besides." }),
            ItemModule::Bound,
        ],
    },
    TobbsLetter: ItemDef {
        name: "Tobb's Letter",
        icon: AssetRef("icons/misc/envolop.png"),
        sfx: ItemSfx { pickup: SfxId::PickupPaper, trade: SfxId::TradePaper, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Sealed with candle wax and a fishy thumbprint, for Captain Bram of the Gull." }),
            ItemModule::QuestItem,
        ],
    },
    TatteredMap: ItemDef {
        name: "Tattered Map",
        icon: AssetRef("icons/misc/map.png"),
        sfx: ItemSfx { pickup: SfxId::PickupPaper, trade: SfxId::TradePaper, drop: SfxId::Landing01, on_use: Some(SfxId::UiPage) },
        kind: ItemKind::Usable { then: &[&OfferQuest(QuestId::XMarksTheSpot)] },
        modules: &[
            ItemModule::Flavor(Flavor { text: "Wrapped in oilcloth and left by the tide. Someone inked a cross deep in the forest." }),
            ItemModule::QuestItem,
        ],
    },
    BelfryKey: ItemDef {
        name: "Belfry Key",
        icon: AssetRef("icons/misc/iron_key.png"),
        sfx: ItemSfx { pickup: SfxId::PickupTrinket, trade: SfxId::TradeTrinket, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Heavy, cold, and nibbled at the bow. Bram wants it back." }),
            ItemModule::QuestItem,
        ],
    },
    FishingBait: ItemDef {
        name: "Fishing Bait",
        icon: AssetRef("icons/monster_part/monster_meat.png"),
        sfx: ItemSfx { pickup: SfxId::PickupFlesh, trade: SfxId::TradeFlesh, drop: SfxId::Landing01, on_use: None },
        kind: ItemKind::Resource,
        modules: &[
            ItemModule::Flavor(Flavor { text: "Smells exactly as bad as you'd think. The fish disagree." }),
            ItemModule::Stack(Stack { max: 20 }),
        ],
    },
    CorsairCutlass: ItemDef {
        name: "Corsair's Cutlass",
        icon: AssetRef("icons/weapon_and_tool/golden_sword.png"),
        sfx: ItemSfx { pickup: SfxId::PickupBlade, trade: SfxId::TradeBlade, drop: SfxId::Block01, on_use: None },
        kind: ItemKind::Equipment {
            slot: EquipmentSlot::Weapon,
            requires: &[&MinLevel(3)],
        },
        modules: &[
            ItemModule::Flavor(Flavor { text: "A gilded blade from a ship nobody remembers. Worth following the cross for." }),
            ItemModule::Stat(Stat { kind: StatKind::Damage, value: 5.0 }),
        ],
    },
}
