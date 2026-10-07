use crate::core::assets::AssetRef;
use crate::data::expression::Id as ExpressionId;
use crate::systems::actor::bust::{Busts, GenericBusts, ModelDef};

crate::table! {
    Adventurer: ModelDef {
        sheet: AssetRef("models/adventurer.tsx"),
        busts: None,
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
            individual: &[ExpressionId::TobbLaughing],
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
            individual: &[ExpressionId::PellSmirk],
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
            individual: &[ExpressionId::MaraSmug, ExpressionId::MaraCounting],
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
            individual: &[ExpressionId::WrenSleepy],
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
