use crate::core::assets::AssetRef;
use crate::systems::interface::{CursorDef, Hotspot};

crate::table! {
    Default: CursorDef { image: AssetRef("icons/cursors/pointer003.png"), hotspot: Hotspot { x: 0, y: 0 } },
    Pointer: CursorDef { image: AssetRef("icons/cursors/hand002.png"), hotspot: Hotspot { x: 3, y: 3 } },
    Grab: CursorDef { image: AssetRef("icons/cursors/hand001.png"), hotspot: Hotspot { x: 28, y: 30 } },
    Grabbing: CursorDef { image: AssetRef("icons/cursors/hand003.png"), hotspot: Hotspot { x: 32, y: 30 } },
    Resize: CursorDef { image: AssetRef("icons/cursors/move006.png"), hotspot: Hotspot { x: 32, y: 31 } },
    Walk: CursorDef { image: AssetRef("icons/cursors/pointer010.png"), hotspot: Hotspot { x: 32, y: 32 } },
    WalkHeld: CursorDef { image: AssetRef("icons/cursors/pointer011.png"), hotspot: Hotspot { x: 32, y: 32 } },
    Attack: CursorDef { image: AssetRef("icons/cursors/swords002.png"), hotspot: Hotspot { x: 32, y: 32 } },
    PickUp: CursorDef { image: AssetRef("icons/cursors/hand001.png"), hotspot: Hotspot { x: 8, y: 8 } },
    Talk: CursorDef { image: AssetRef("icons/cursors/talk001.png"), hotspot: Hotspot { x: 4, y: 4 } },
    Use: CursorDef { image: AssetRef("icons/cursors/hand002.png"), hotspot: Hotspot { x: 4, y: 4 } },
    Open: CursorDef { image: AssetRef("icons/cursors/chest001.png"), hotspot: Hotspot { x: 4, y: 4 } },
    Read: CursorDef { image: AssetRef("icons/cursors/book001.png"), hotspot: Hotspot { x: 4, y: 4 } },
}
