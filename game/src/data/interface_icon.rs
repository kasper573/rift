use crate::core::assets::AssetRef;
use crate::systems::interface::InterfaceIconDef;

crate::table! {
    Settings: InterfaceIconDef { image: AssetRef("icons/misc/gear.png") },
    Bag: InterfaceIconDef { image: AssetRef("icons/equipment/bag.png") },
    Equipment: InterfaceIconDef { image: AssetRef("icons/equipment/helm.png") },
    Stats: InterfaceIconDef { image: AssetRef("icons/misc/book.png") },
    QuestLog: InterfaceIconDef { image: AssetRef("icons/misc/book.png") },
    History: InterfaceIconDef { image: AssetRef("icons/misc/book_3.png") },
    Checked: InterfaceIconDef { image: AssetRef("icons/misc/checkmark.png") },
    Locked: InterfaceIconDef { image: AssetRef("icons/cursors/lock001.png") },
    Tracked: InterfaceIconDef { image: AssetRef("icons/cursors/eye001.png") },
    Timed: InterfaceIconDef { image: AssetRef("icons/cursors/sandclock001.png") },
    ObjectiveDefeat: InterfaceIconDef { image: AssetRef("icons/cursors/swords001.png") },
    ObjectiveTask: InterfaceIconDef { image: AssetRef("icons/misc/map.png") },
    ObjectiveDecide: InterfaceIconDef { image: AssetRef("icons/cursors/question001.png") },
    WalkTarget: InterfaceIconDef { image: AssetRef("icons/crosshairs/white/crosshair026.png") },
    Terminal: InterfaceIconDef { image: AssetRef("icons/misc/scroll.png") },
}
