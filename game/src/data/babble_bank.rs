use crate::core::assets::AssetRef;
use crate::core::babble::BabbleBankDef;

crate::table! {
    Soft: BabbleBankDef { letters: AssetRef("sfx/babble/soft"), volume: 0.5 },
    Gruff: BabbleBankDef { letters: AssetRef("sfx/babble/gruff"), volume: 0.6 },
}
