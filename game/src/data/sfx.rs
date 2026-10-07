use crate::core::assets::AssetRef;
use crate::core::audio::playback::{SfxDef, SfxScalar};

crate::table! {
    Bite01: SfxDef {
        src: AssetRef("audio/combat/bite01.wav"),
        volume: SfxScalar::Random(0.57, 0.71),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Block01: SfxDef {
        src: AssetRef("audio/combat/block01.wav"),
        volume: SfxScalar::Random(0.81, 1.01),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Climb01: SfxDef {
        src: AssetRef("audio/movement/climb01.wav"),
        volume: SfxScalar::Random(1.34, 1.68),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Death01: SfxDef {
        src: AssetRef("audio/combat/death01.wav"),
        volume: SfxScalar::Random(0.63, 0.79),
        pitch: SfxScalar::Fixed(1.0),
    },
    Dodge01: SfxDef {
        src: AssetRef("audio/combat/dodge01.wav"),
        volume: SfxScalar::Random(0.62, 0.77),
        pitch: SfxScalar::Fixed(1.0),
    },
    Heal01: SfxDef {
        src: AssetRef("audio/buffs/heal01.wav"),
        volume: SfxScalar::Random(0.18, 0.22),
        pitch: SfxScalar::Fixed(1.0),
    },
    Jump01: SfxDef {
        src: AssetRef("audio/movement/jump01.wav"),
        volume: SfxScalar::Random(0.79, 0.99),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Landing01: SfxDef {
        src: AssetRef("audio/movement/landing01.wav"),
        volume: SfxScalar::Random(0.61, 0.76),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash01: SfxDef {
        src: AssetRef("audio/combat/slash01.wav"),
        volume: SfxScalar::Random(0.5, 0.63),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash02: SfxDef {
        src: AssetRef("audio/combat/slash02.wav"),
        volume: SfxScalar::Random(0.66, 0.83),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Slash03: SfxDef {
        src: AssetRef("audio/combat/slash03.wav"),
        volume: SfxScalar::Random(1.34, 1.68),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepGrass01: SfxDef {
        src: AssetRef("audio/movement/step_grass01.wav"),
        volume: SfxScalar::Random(0.72, 0.9),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepRock01: SfxDef {
        src: AssetRef("audio/movement/step_rock01.wav"),
        volume: SfxScalar::Random(0.79, 0.99),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepWood01: SfxDef {
        src: AssetRef("audio/movement/step_wood01.wav"),
        volume: SfxScalar::Random(0.76, 0.95),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    StepSand01: SfxDef {
        src: AssetRef("audio/movement/step_sand01.wav"),
        volume: SfxScalar::Random(0.28, 0.35),
        pitch: SfxScalar::Random(0.8, 1.2),
    },
    Teleport01: SfxDef {
        src: AssetRef("audio/movement/teleport01.wav"),
        volume: SfxScalar::Random(0.54, 0.68),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiOpen: SfxDef {
        src: AssetRef("audio/interface/open.wav"),
        volume: SfxScalar::Fixed(0.5),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiClose: SfxDef {
        src: AssetRef("audio/interface/close.wav"),
        volume: SfxScalar::Fixed(0.3),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiMove: SfxDef {
        src: AssetRef("audio/interface/move.wav"),
        volume: SfxScalar::Fixed(0.47),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiPick: SfxDef {
        src: AssetRef("audio/interface/pick.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiRefuse: SfxDef {
        src: AssetRef("audio/interface/refuse.wav"),
        volume: SfxScalar::Fixed(0.39),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiChime: SfxDef {
        src: AssetRef("audio/interface/chime.wav"),
        volume: SfxScalar::Fixed(0.47),
        pitch: SfxScalar::Fixed(1.0),
    },
    ForestDrums: SfxDef {
        src: AssetRef("audio/interface/forest_drums.wav"),
        volume: SfxScalar::Fixed(0.11),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    UiPage: SfxDef {
        src: AssetRef("audio/interface/page.wav"),
        volume: SfxScalar::Fixed(0.37),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    Coins: SfxDef {
        src: AssetRef("audio/interface/coins.wav"),
        volume: SfxScalar::Fixed(0.46),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    QuestAccepted: SfxDef {
        src: AssetRef("audio/interface/quest_accepted.wav"),
        volume: SfxScalar::Fixed(0.29),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    QuestCompleted: SfxDef {
        src: AssetRef("audio/interface/quest_completed.wav"),
        volume: SfxScalar::Fixed(0.3),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    QuestAbandoned: SfxDef {
        src: AssetRef("audio/interface/quest_abandoned.wav"),
        volume: SfxScalar::Fixed(0.28),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    RisingChime: SfxDef {
        src: AssetRef("audio/interface/rising_chime.wav"),
        volume: SfxScalar::Fixed(0.15),
        pitch: SfxScalar::Fixed(1.0),
    },
    #[expose]
    TallyTick: SfxDef {
        src: AssetRef("audio/interface/tally_tick.wav"),
        volume: SfxScalar::Fixed(0.23),
        pitch: SfxScalar::Fixed(1.0),
    },
    GullCries: SfxDef {
        src: AssetRef("audio/ambient/gulls.wav"),
        volume: SfxScalar::Fixed(0.13),
        pitch: SfxScalar::Fixed(1.0),
    },
    HullScrape: SfxDef {
        src: AssetRef("audio/ambient/hull_scrape.wav"),
        volume: SfxScalar::Fixed(0.11),
        pitch: SfxScalar::Fixed(1.0),
    },
    PickupCoins: SfxDef {
        src: AssetRef("audio/items/pickup/coins.wav"),
        volume: SfxScalar::Fixed(0.14),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupGlass: SfxDef {
        src: AssetRef("audio/items/pickup/glass.wav"),
        volume: SfxScalar::Fixed(0.14),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupBone: SfxDef {
        src: AssetRef("audio/items/pickup/bone.wav"),
        volume: SfxScalar::Fixed(0.35),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupFlesh: SfxDef {
        src: AssetRef("audio/items/pickup/flesh.wav"),
        volume: SfxScalar::Fixed(0.22),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupBlade: SfxDef {
        src: AssetRef("audio/items/pickup/blade.wav"),
        volume: SfxScalar::Fixed(0.19),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupTrinket: SfxDef {
        src: AssetRef("audio/items/pickup/trinket.wav"),
        volume: SfxScalar::Fixed(0.22),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupPaper: SfxDef {
        src: AssetRef("audio/items/pickup/paper.wav"),
        volume: SfxScalar::Fixed(0.11),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    PickupStone: SfxDef {
        src: AssetRef("audio/items/pickup/stone.wav"),
        volume: SfxScalar::Fixed(0.26),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeCoins: SfxDef {
        src: AssetRef("audio/items/trade/coins.wav"),
        volume: SfxScalar::Fixed(0.16),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeGlass: SfxDef {
        src: AssetRef("audio/items/trade/glass.wav"),
        volume: SfxScalar::Fixed(0.16),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeBone: SfxDef {
        src: AssetRef("audio/items/trade/bone.wav"),
        volume: SfxScalar::Fixed(0.34),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeFlesh: SfxDef {
        src: AssetRef("audio/items/trade/flesh.wav"),
        volume: SfxScalar::Fixed(0.28),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeBlade: SfxDef {
        src: AssetRef("audio/items/trade/blade.wav"),
        volume: SfxScalar::Fixed(0.18),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeTrinket: SfxDef {
        src: AssetRef("audio/items/trade/trinket.wav"),
        volume: SfxScalar::Fixed(0.2),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradePaper: SfxDef {
        src: AssetRef("audio/items/trade/paper.wav"),
        volume: SfxScalar::Fixed(0.18),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
    TradeStone: SfxDef {
        src: AssetRef("audio/items/trade/stone.wav"),
        volume: SfxScalar::Fixed(0.33),
        pitch: SfxScalar::Random(0.95, 1.05),
    },
}
