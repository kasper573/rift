use crate::core::assets::AssetRef;
use crate::systems::actor::bust::{Busts, GenericBusts, IndividualExpression, ModelDef};

crate::table! {
    Adventurer: ModelDef {
        sheet: AssetRef("models/adventurer.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/adventurer/neutral.png"),
                happy: AssetRef("busts/adventurer/happy.png"),
                sad: AssetRef("busts/adventurer/sad.png"),
                angry: AssetRef("busts/adventurer/angry.png"),
                surprised: AssetRef("busts/adventurer/surprised.png"),
                thinking: AssetRef("busts/adventurer/thinking.png"),
            },
            individual: &[],
        }),
    },
    Tobb: ModelDef {
        sheet: AssetRef("models/tobb.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/tobb/neutral.png"),
                happy: AssetRef("busts/tobb/happy.png"),
                sad: AssetRef("busts/tobb/sad.png"),
                angry: AssetRef("busts/tobb/angry.png"),
                surprised: AssetRef("busts/tobb/surprised.png"),
                thinking: AssetRef("busts/tobb/thinking.png"),
            },
            individual: &[
                (IndividualExpression::Laughing, AssetRef("busts/tobb/laughing.png")),
            ],
        }),
    },
    Bram: ModelDef {
        sheet: AssetRef("models/bram.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/bram/neutral.png"),
                happy: AssetRef("busts/bram/happy.png"),
                sad: AssetRef("busts/bram/sad.png"),
                angry: AssetRef("busts/bram/angry.png"),
                surprised: AssetRef("busts/bram/surprised.png"),
                thinking: AssetRef("busts/bram/thinking.png"),
            },
            individual: &[],
        }),
    },
    Pell: ModelDef {
        sheet: AssetRef("models/pell.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/pell/neutral.png"),
                happy: AssetRef("busts/pell/happy.png"),
                sad: AssetRef("busts/pell/sad.png"),
                angry: AssetRef("busts/pell/angry.png"),
                surprised: AssetRef("busts/pell/surprised.png"),
                thinking: AssetRef("busts/pell/thinking.png"),
            },
            individual: &[
                (IndividualExpression::Smirk, AssetRef("busts/pell/smirk.png")),
            ],
        }),
    },
    Mara: ModelDef {
        sheet: AssetRef("models/mara.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/mara/neutral.png"),
                happy: AssetRef("busts/mara/happy.png"),
                sad: AssetRef("busts/mara/sad.png"),
                angry: AssetRef("busts/mara/angry.png"),
                surprised: AssetRef("busts/mara/surprised.png"),
                thinking: AssetRef("busts/mara/thinking.png"),
            },
            individual: &[
                (IndividualExpression::Smug, AssetRef("busts/mara/smug.png")),
                (IndividualExpression::Counting, AssetRef("busts/mara/counting.png")),
            ],
        }),
    },
    Grisha: ModelDef {
        sheet: AssetRef("models/grisha.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/grisha/neutral.png"),
                happy: AssetRef("busts/grisha/happy.png"),
                sad: AssetRef("busts/grisha/sad.png"),
                angry: AssetRef("busts/grisha/angry.png"),
                surprised: AssetRef("busts/grisha/surprised.png"),
                thinking: AssetRef("busts/grisha/thinking.png"),
            },
            individual: &[],
        }),
    },
    Wren: ModelDef {
        sheet: AssetRef("models/wren.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/wren/neutral.png"),
                happy: AssetRef("busts/wren/happy.png"),
                sad: AssetRef("busts/wren/sad.png"),
                angry: AssetRef("busts/wren/angry.png"),
                surprised: AssetRef("busts/wren/surprised.png"),
                thinking: AssetRef("busts/wren/thinking.png"),
            },
            individual: &[
                (IndividualExpression::Sleepy, AssetRef("busts/wren/sleepy.png")),
            ],
        }),
    },
    Ilsa: ModelDef {
        sheet: AssetRef("models/ilsa.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/ilsa/neutral.png"),
                happy: AssetRef("busts/ilsa/happy.png"),
                sad: AssetRef("busts/ilsa/sad.png"),
                angry: AssetRef("busts/ilsa/angry.png"),
                surprised: AssetRef("busts/ilsa/surprised.png"),
                thinking: AssetRef("busts/ilsa/thinking.png"),
            },
            individual: &[],
        }),
    },
    Ugra: ModelDef {
        sheet: AssetRef("models/ugra.tsx"),
        busts: Some(Busts {
            generic: GenericBusts {
                neutral: AssetRef("busts/ugra/neutral.png"),
                happy: AssetRef("busts/ugra/happy.png"),
                sad: AssetRef("busts/ugra/sad.png"),
                angry: AssetRef("busts/ugra/angry.png"),
                surprised: AssetRef("busts/ugra/surprised.png"),
                thinking: AssetRef("busts/ugra/thinking.png"),
            },
            individual: &[],
        }),
    },
    Guard: ModelDef { sheet: AssetRef("models/guard.tsx"), busts: None },
    Bat: ModelDef { sheet: AssetRef("models/bat.tsx"), busts: None },
    Orc: ModelDef { sheet: AssetRef("models/orc.tsx"), busts: None },
    Skeleton: ModelDef { sheet: AssetRef("models/skeleton.tsx"), busts: None },
}
