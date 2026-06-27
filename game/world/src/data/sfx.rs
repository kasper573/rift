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
}
