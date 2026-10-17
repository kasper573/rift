use std::collections::HashMap;
use std::sync::Arc;

use bevy_ecs::world::World;
use design_tokens::ThemeFonts;
use egui::{FontData, FontDefinitions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::audio::playback::{PlaySfx, SfxDef, SfxPlace};
use crate::core::content::{Content, ContentRow, Fixture};

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct CursorDef {
    pub image: AssetRef,
    pub hotspot: Hotspot,
}

impl ContentRow for CursorDef {
    const TABLE: &'static str = "cursor";
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotspot {
    #[schemars(extend("x-unit" = "px"))]
    pub x: u16,
    #[schemars(extend("x-unit" = "px"))]
    pub y: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum CursorShape {
    Default,
    Pointer,
    Grab,
    Grabbing,
    Resize,
    Walk,
    WalkHeld,
    Attack,
    PickUp,
    Talk,
    Use,
    Open,
    Read,
}

impl Fixture for CursorShape {
    type Row = CursorDef;
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InterfaceIconDef {
    pub image: AssetRef,
}

impl ContentRow for InterfaceIconDef {
    const TABLE: &'static str = "interface_icon";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum InterfaceIcon {
    Settings,
    Bag,
    Equipment,
    Stats,
    QuestLog,
    History,
    Checked,
    Locked,
    Tracked,
    Timed,
    ObjectiveDefeat,
    ObjectiveTask,
    ObjectiveDecide,
    WalkTarget,
    Terminal,
    Skills,
}

impl Fixture for InterfaceIcon {
    type Row = InterfaceIconDef;
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct FontDef {
    pub file: AssetRef,
}

impl ContentRow for FontDef {
    const TABLE: &'static str = "font";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
pub enum FontFace {
    #[strum(serialize = "circular_400")]
    Circular400,
    #[strum(serialize = "circular_500")]
    Circular500,
    #[strum(serialize = "circular_700")]
    Circular700,
    #[strum(serialize = "lato_400")]
    Lato400,
    #[strum(serialize = "lato_400_italic")]
    Lato400Italic,
    #[strum(serialize = "lato_700")]
    Lato700,
    #[strum(serialize = "lato_700_italic")]
    Lato700Italic,
}

impl Fixture for FontFace {
    type Row = FontDef;
}

pub fn theme_fonts(
    content: &Content,
    files: &HashMap<String, Arc<[u8]>>,
) -> Option<FontDefinitions> {
    let face = |face: FontFace| -> Option<FontData> {
        let bytes = files.get(&face.get(content).file.path())?;
        Some(FontData::from_owned(bytes.to_vec()))
    };
    Some(design_tokens::fonts(ThemeFonts {
        circular_400: face(FontFace::Circular400)?,
        circular_500: face(FontFace::Circular500)?,
        circular_700: face(FontFace::Circular700)?,
        lato_400: face(FontFace::Lato400)?,
        lato_400_italic: face(FontFace::Lato400Italic)?,
        lato_700: face(FontFace::Lato700)?,
        lato_700_italic: face(FontFace::Lato700Italic)?,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum InterfaceSound {
    UiOpen,
    UiClose,
    UiMove,
    UiPick,
    UiRefuse,
    UiChime,
    UiPage,
    Coins,
    QuestAccepted,
    QuestCompleted,
    QuestAbandoned,
    RisingChime,
    TallyTick,
}

impl Fixture for InterfaceSound {
    type Row = SfxDef;
}

impl InterfaceSound {
    pub fn play(self, world: &mut World) {
        let id = self.id(world.resource::<Content>());
        world.write_message(PlaySfx {
            id,
            place: SfxPlace::Interface,
        });
    }
}
