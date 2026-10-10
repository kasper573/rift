use crate::core::assets::AssetRef;
use crate::systems::interface::FontDef;

crate::table! {
    Circular_400: FontDef { file: AssetRef("fonts/circular-400-normal.ttf") },
    Circular_500: FontDef { file: AssetRef("fonts/circular-500-normal.ttf") },
    Circular_700: FontDef { file: AssetRef("fonts/circular-700-normal.ttf") },
    Lato_400: FontDef { file: AssetRef("fonts/lato-400-normal.ttf") },
    Lato_400_italic: FontDef { file: AssetRef("fonts/lato-400-italic.ttf") },
    Lato_700: FontDef { file: AssetRef("fonts/lato-700-normal.ttf") },
    Lato_700_italic: FontDef { file: AssetRef("fonts/lato-700-italic.ttf") },
}
