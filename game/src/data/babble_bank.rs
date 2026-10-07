use crate::core::assets::AssetRef;
use crate::core::babble::BabbleBankDef;

crate::table! {
    Soft: BabbleBankDef { letters: AssetRef("audio/babble/soft"), volume: 0.2 },
    Gruff: BabbleBankDef { letters: AssetRef("audio/babble/gruff"), volume: 0.22 },
}
