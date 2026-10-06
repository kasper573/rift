use crate::core::assets::AssetRef;
use crate::core::sfx::{SfxDef, SfxScalar};

crate::table! {
    Bite01: SfxDef {
        src: AssetRef("sfx/combat/bite01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Block01: SfxDef {
        src: AssetRef("sfx/combat/block01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Climb01: SfxDef {
        src: AssetRef("sfx/movement/climb01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Death01: SfxDef {
        src: AssetRef("sfx/combat/death01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    Dodge01: SfxDef {
        src: AssetRef("sfx/combat/dodge01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    Heal01: SfxDef {
        src: AssetRef("sfx/buffs/heal01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    Jump01: SfxDef {
        src: AssetRef("sfx/movement/jump01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Landing01: SfxDef {
        src: AssetRef("sfx/movement/landing01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash01: SfxDef {
        src: AssetRef("sfx/combat/slash01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash02: SfxDef {
        src: AssetRef("sfx/combat/slash02.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash03: SfxDef {
        src: AssetRef("sfx/combat/slash03.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepGrass01: SfxDef {
        src: AssetRef("sfx/movement/step_grass01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepRock01: SfxDef {
        src: AssetRef("sfx/movement/step_rock01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepWood01: SfxDef {
        src: AssetRef("sfx/movement/step_wood01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepSand01: SfxDef {
        src: AssetRef("sfx/movement/step_sand01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Teleport01: SfxDef {
        src: AssetRef("sfx/movement/teleport01.wav"),
        volume: SfxScalar::Random(0.8, 1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiOpen: SfxDef {
        src: AssetRef("sfx/interface/open.wav"),
        volume: SfxScalar::Fixed(0.3),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiClose: SfxDef {
        src: AssetRef("sfx/interface/close.wav"),
        volume: SfxScalar::Fixed(0.3),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiMove: SfxDef {
        src: AssetRef("sfx/interface/move.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiPick: SfxDef {
        src: AssetRef("sfx/interface/pick.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiRefuse: SfxDef {
        src: AssetRef("sfx/interface/refuse.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiChime: SfxDef {
        src: AssetRef("sfx/interface/chime.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    ForestDrums: SfxDef {
        src: AssetRef("sfx/interface/forest_drums.wav"),
        volume: SfxScalar::Fixed(0.8),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiPage: SfxDef {
        src: AssetRef("sfx/interface/page.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    Coins: SfxDef {
        src: AssetRef("sfx/interface/coins.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    QuestAccepted: SfxDef {
        src: AssetRef("sfx/interface/quest_accepted.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    QuestCompleted: SfxDef {
        src: AssetRef("sfx/interface/quest_completed.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    QuestAbandoned: SfxDef {
        src: AssetRef("sfx/interface/quest_abandoned.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    RisingChime: SfxDef {
        src: AssetRef("sfx/interface/rising_chime.wav"),
        volume: SfxScalar::Fixed(0.7),
        pitch: SfxScalar::Fixed(1.0),
    },
    TallyTick: SfxDef {
        src: AssetRef("sfx/interface/tally_tick.wav"),
        volume: SfxScalar::Fixed(0.4),
        pitch: SfxScalar::Fixed(1.0),
    },
    GullCries: SfxDef {
        src: AssetRef("sfx/ambient/gulls.wav"),
        volume: SfxScalar::Fixed(0.4),
        pitch: SfxScalar::Fixed(1.0),
    },
    HullScrape: SfxDef {
        src: AssetRef("sfx/ambient/hull_scrape.wav"),
        volume: SfxScalar::Fixed(0.5),
        pitch: SfxScalar::Fixed(1.0),
    },
    PickupCoins: SfxDef {
        src: AssetRef("sfx/items/pickup/coins.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupGlass: SfxDef {
        src: AssetRef("sfx/items/pickup/glass.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupBone: SfxDef {
        src: AssetRef("sfx/items/pickup/bone.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupFlesh: SfxDef {
        src: AssetRef("sfx/items/pickup/flesh.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupBlade: SfxDef {
        src: AssetRef("sfx/items/pickup/blade.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupTrinket: SfxDef {
        src: AssetRef("sfx/items/pickup/trinket.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupPaper: SfxDef {
        src: AssetRef("sfx/items/pickup/paper.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupStone: SfxDef {
        src: AssetRef("sfx/items/pickup/stone.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeCoins: SfxDef {
        src: AssetRef("sfx/items/trade/coins.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeGlass: SfxDef {
        src: AssetRef("sfx/items/trade/glass.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeBone: SfxDef {
        src: AssetRef("sfx/items/trade/bone.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeFlesh: SfxDef {
        src: AssetRef("sfx/items/trade/flesh.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeBlade: SfxDef {
        src: AssetRef("sfx/items/trade/blade.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeTrinket: SfxDef {
        src: AssetRef("sfx/items/trade/trinket.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradePaper: SfxDef {
        src: AssetRef("sfx/items/trade/paper.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeStone: SfxDef {
        src: AssetRef("sfx/items/trade/stone.wav"),
        volume: SfxScalar::Fixed(0.55),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
}
