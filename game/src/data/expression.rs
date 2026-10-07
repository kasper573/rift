use crate::core::assets::AssetRef;
use crate::systems::actor::bust::IndividualExpressionDef;

crate::table! {
    TobbLaughing: IndividualExpressionDef { bust: AssetRef("busts/tobb/laughing.png") },
    PellSmirk: IndividualExpressionDef { bust: AssetRef("busts/pell/smirk.png") },
    MaraSmug: IndividualExpressionDef { bust: AssetRef("busts/mara/smug.png") },
    MaraCounting: IndividualExpressionDef { bust: AssetRef("busts/mara/counting.png") },
    WrenSleepy: IndividualExpressionDef { bust: AssetRef("busts/wren/sleepy.png") },
}
