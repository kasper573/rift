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
    AdventurerHm: SfxDef {
        src: AssetRef("sfx/voice/low/mmhmm_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    AdventurerLaugh: SfxDef {
        src: AssetRef("sfx/voice/low/haha_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    AdventurerSigh: SfxDef {
        src: AssetRef("sfx/voice/low/sigh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    AdventurerGrunt: SfxDef {
        src: AssetRef("sfx/voice/low/grr_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    AdventurerGasp: SfxDef {
        src: AssetRef("sfx/voice/low/gasp_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    AdventurerHmm: SfxDef {
        src: AssetRef("sfx/voice/low/hmm_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    TobbHm: SfxDef {
        src: AssetRef("sfx/voice/low/uhuh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbLaugh: SfxDef {
        src: AssetRef("sfx/voice/low/laugh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbSigh: SfxDef {
        src: AssetRef("sfx/voice/low/phew_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbGrunt: SfxDef {
        src: AssetRef("sfx/voice/low/ugh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbGasp: SfxDef {
        src: AssetRef("sfx/voice/low/woah_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbHmm: SfxDef {
        src: AssetRef("sfx/voice/low/hmm_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    TobbHaha: SfxDef {
        src: AssetRef("sfx/voice/low/haha_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.1),
    },
    BramHm: SfxDef {
        src: AssetRef("sfx/voice/low/mmhmm_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    BramLaugh: SfxDef {
        src: AssetRef("sfx/voice/low/laugh_03.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    BramSigh: SfxDef {
        src: AssetRef("sfx/voice/low/sigh_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    BramGrunt: SfxDef {
        src: AssetRef("sfx/voice/low/grunt_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    BramGasp: SfxDef {
        src: AssetRef("sfx/voice/low/surprised_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    BramHmm: SfxDef {
        src: AssetRef("sfx/voice/low/hmm_03.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.85),
    },
    PellHm: SfxDef {
        src: AssetRef("sfx/voice/low/huh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellLaugh: SfxDef {
        src: AssetRef("sfx/voice/low/haha_03.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellSigh: SfxDef {
        src: AssetRef("sfx/voice/low/breath_out_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellGrunt: SfxDef {
        src: AssetRef("sfx/voice/low/grr_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellGasp: SfxDef {
        src: AssetRef("sfx/voice/low/gasp_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellHmm: SfxDef {
        src: AssetRef("sfx/voice/low/hmm_04.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    PellHmph: SfxDef {
        src: AssetRef("sfx/voice/low/hmph_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.95),
    },
    MaraHm: SfxDef {
        src: AssetRef("sfx/voice/mature/eh.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraLaugh: SfxDef {
        src: AssetRef("sfx/voice/mature/laugh.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraSigh: SfxDef {
        src: AssetRef("sfx/voice/mature/sigh_01.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraGrunt: SfxDef {
        src: AssetRef("sfx/voice/mature/grumble.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraGasp: SfxDef {
        src: AssetRef("sfx/voice/mature/gasp.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraHmm: SfxDef {
        src: AssetRef("sfx/voice/mature/uh.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    MaraHmph: SfxDef {
        src: AssetRef("sfx/voice/mature/chuckle.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    GrishaHm: SfxDef {
        src: AssetRef("sfx/voice/mature/ah.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    GrishaLaugh: SfxDef {
        src: AssetRef("sfx/voice/mature/laugh.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    GrishaSigh: SfxDef {
        src: AssetRef("sfx/voice/mature/sigh_02.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    GrishaGrunt: SfxDef {
        src: AssetRef("sfx/voice/mature/grumble.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    GrishaGasp: SfxDef {
        src: AssetRef("sfx/voice/mature/wah.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    GrishaHmm: SfxDef {
        src: AssetRef("sfx/voice/mature/haa.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.9),
    },
    WrenHm: SfxDef {
        src: AssetRef("sfx/voice/bright/okay.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenLaugh: SfxDef {
        src: AssetRef("sfx/voice/bright/laughter.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenSigh: SfxDef {
        src: AssetRef("sfx/voice/bright/why.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenGrunt: SfxDef {
        src: AssetRef("sfx/voice/bright/what.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenGasp: SfxDef {
        src: AssetRef("sfx/voice/bright/anime_gasp.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenHmm: SfxDef {
        src: AssetRef("sfx/voice/bright/actually.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    WrenYawn: SfxDef {
        src: AssetRef("sfx/voice/bright/okay.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(0.8),
    },
    IlsaHm: SfxDef {
        src: AssetRef("sfx/voice/clear/okay.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    IlsaLaugh: SfxDef {
        src: AssetRef("sfx/voice/clear/laughter.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    IlsaSigh: SfxDef {
        src: AssetRef("sfx/voice/clear/why.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    IlsaGrunt: SfxDef {
        src: AssetRef("sfx/voice/clear/what.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    IlsaGasp: SfxDef {
        src: AssetRef("sfx/voice/clear/anime_gasp.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    IlsaHmm: SfxDef {
        src: AssetRef("sfx/voice/clear/actually.wav"),
        volume: SfxScalar::Fixed(0.9),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiOpen: SfxDef {
        src: AssetRef("sfx/interface/open.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiClose: SfxDef {
        src: AssetRef("sfx/interface/close.wav"),
        volume: SfxScalar::Fixed(1.0),
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
    UiToast: SfxDef {
        src: AssetRef("sfx/interface/toast.wav"),
        volume: SfxScalar::Fixed(1.0),
        pitch: SfxScalar::Fixed(1.0),
    },
    UiChime: SfxDef {
        src: AssetRef("sfx/interface/chime.wav"),
        volume: SfxScalar::Fixed(1.0),
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
}
