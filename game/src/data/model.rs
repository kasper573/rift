use crate::core::assets::AssetRef;
use crate::core::sfx::SfxId;
use crate::systems::actor::bust::{Bust, Busts, GenericBusts, IndividualExpression, ModelDef};

crate::table! {
    Adventurer: ModelDef {
        sheet: AssetRef("models/adventurer.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/adventurer/neutral.png"), cue: SfxId::AdventurerHm },
                happy: Bust { art: AssetRef("busts/adventurer/happy.png"), cue: SfxId::AdventurerLaugh },
                sad: Bust { art: AssetRef("busts/adventurer/sad.png"), cue: SfxId::AdventurerSigh },
                angry: Bust { art: AssetRef("busts/adventurer/angry.png"), cue: SfxId::AdventurerGrunt },
                surprised: Bust { art: AssetRef("busts/adventurer/surprised.png"), cue: SfxId::AdventurerGasp },
                thinking: Bust { art: AssetRef("busts/adventurer/thinking.png"), cue: SfxId::AdventurerHmm },
            },
            individual: &[],
        }),
    },
    Tobb: ModelDef {
        sheet: AssetRef("models/tobb.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/tobb/neutral.png"), cue: SfxId::TobbHm },
                happy: Bust { art: AssetRef("busts/tobb/happy.png"), cue: SfxId::TobbLaugh },
                sad: Bust { art: AssetRef("busts/tobb/sad.png"), cue: SfxId::TobbSigh },
                angry: Bust { art: AssetRef("busts/tobb/angry.png"), cue: SfxId::TobbGrunt },
                surprised: Bust { art: AssetRef("busts/tobb/surprised.png"), cue: SfxId::TobbGasp },
                thinking: Bust { art: AssetRef("busts/tobb/thinking.png"), cue: SfxId::TobbHmm },
            },
            individual: &[
                (IndividualExpression::Laughing, Bust { art: AssetRef("busts/tobb/laughing.png"), cue: SfxId::TobbHaha }),
            ],
        }),
    },
    Bram: ModelDef {
        sheet: AssetRef("models/bram.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/bram/neutral.png"), cue: SfxId::BramHm },
                happy: Bust { art: AssetRef("busts/bram/happy.png"), cue: SfxId::BramLaugh },
                sad: Bust { art: AssetRef("busts/bram/sad.png"), cue: SfxId::BramSigh },
                angry: Bust { art: AssetRef("busts/bram/angry.png"), cue: SfxId::BramGrunt },
                surprised: Bust { art: AssetRef("busts/bram/surprised.png"), cue: SfxId::BramGasp },
                thinking: Bust { art: AssetRef("busts/bram/thinking.png"), cue: SfxId::BramHmm },
            },
            individual: &[],
        }),
    },
    Pell: ModelDef {
        sheet: AssetRef("models/pell.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/pell/neutral.png"), cue: SfxId::PellHm },
                happy: Bust { art: AssetRef("busts/pell/happy.png"), cue: SfxId::PellLaugh },
                sad: Bust { art: AssetRef("busts/pell/sad.png"), cue: SfxId::PellSigh },
                angry: Bust { art: AssetRef("busts/pell/angry.png"), cue: SfxId::PellGrunt },
                surprised: Bust { art: AssetRef("busts/pell/surprised.png"), cue: SfxId::PellGasp },
                thinking: Bust { art: AssetRef("busts/pell/thinking.png"), cue: SfxId::PellHmm },
            },
            individual: &[
                (IndividualExpression::Smirk, Bust { art: AssetRef("busts/pell/smirk.png"), cue: SfxId::PellHmph }),
            ],
        }),
    },
    Mara: ModelDef {
        sheet: AssetRef("models/mara.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/mara/neutral.png"), cue: SfxId::MaraHm },
                happy: Bust { art: AssetRef("busts/mara/happy.png"), cue: SfxId::MaraLaugh },
                sad: Bust { art: AssetRef("busts/mara/sad.png"), cue: SfxId::MaraSigh },
                angry: Bust { art: AssetRef("busts/mara/angry.png"), cue: SfxId::MaraGrunt },
                surprised: Bust { art: AssetRef("busts/mara/surprised.png"), cue: SfxId::MaraGasp },
                thinking: Bust { art: AssetRef("busts/mara/thinking.png"), cue: SfxId::MaraHmm },
            },
            individual: &[
                (IndividualExpression::Smug, Bust { art: AssetRef("busts/mara/smug.png"), cue: SfxId::MaraHmph }),
                (IndividualExpression::Counting, Bust { art: AssetRef("busts/mara/counting.png"), cue: SfxId::Coins }),
            ],
        }),
    },
    Grisha: ModelDef {
        sheet: AssetRef("models/grisha.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/grisha/neutral.png"), cue: SfxId::GrishaHm },
                happy: Bust { art: AssetRef("busts/grisha/happy.png"), cue: SfxId::GrishaLaugh },
                sad: Bust { art: AssetRef("busts/grisha/sad.png"), cue: SfxId::GrishaSigh },
                angry: Bust { art: AssetRef("busts/grisha/angry.png"), cue: SfxId::GrishaGrunt },
                surprised: Bust { art: AssetRef("busts/grisha/surprised.png"), cue: SfxId::GrishaGasp },
                thinking: Bust { art: AssetRef("busts/grisha/thinking.png"), cue: SfxId::GrishaHmm },
            },
            individual: &[],
        }),
    },
    Wren: ModelDef {
        sheet: AssetRef("models/wren.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/wren/neutral.png"), cue: SfxId::WrenHm },
                happy: Bust { art: AssetRef("busts/wren/happy.png"), cue: SfxId::WrenLaugh },
                sad: Bust { art: AssetRef("busts/wren/sad.png"), cue: SfxId::WrenSigh },
                angry: Bust { art: AssetRef("busts/wren/angry.png"), cue: SfxId::WrenGrunt },
                surprised: Bust { art: AssetRef("busts/wren/surprised.png"), cue: SfxId::WrenGasp },
                thinking: Bust { art: AssetRef("busts/wren/thinking.png"), cue: SfxId::WrenHmm },
            },
            individual: &[
                (IndividualExpression::Sleepy, Bust { art: AssetRef("busts/wren/sleepy.png"), cue: SfxId::WrenYawn }),
            ],
        }),
    },
    Ilsa: ModelDef {
        sheet: AssetRef("models/ilsa.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: Bust { art: AssetRef("busts/ilsa/neutral.png"), cue: SfxId::IlsaHm },
                happy: Bust { art: AssetRef("busts/ilsa/happy.png"), cue: SfxId::IlsaLaugh },
                sad: Bust { art: AssetRef("busts/ilsa/sad.png"), cue: SfxId::IlsaSigh },
                angry: Bust { art: AssetRef("busts/ilsa/angry.png"), cue: SfxId::IlsaGrunt },
                surprised: Bust { art: AssetRef("busts/ilsa/surprised.png"), cue: SfxId::IlsaGasp },
                thinking: Bust { art: AssetRef("busts/ilsa/thinking.png"), cue: SfxId::IlsaHmm },
            },
            individual: &[],
        }),
    },
    Guard: ModelDef { sheet: AssetRef("models/guard.tsx"), busts: None },
    Bat: ModelDef { sheet: AssetRef("models/bat.tsx"), busts: None },
    Orc: ModelDef { sheet: AssetRef("models/orc.tsx"), busts: None },
    Skeleton: ModelDef { sheet: AssetRef("models/skeleton.tsx"), busts: None },
}
