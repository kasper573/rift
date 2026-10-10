use strum::VariantArray;

use crate::core::assets::{AssetRef, AssetService};
use crate::core::audio::playback::SfxDef;
use crate::core::content::{Content, ContentRow, Fixture};

#[derive(Clone)]
pub struct CursorDef {
    pub image: AssetRef,
    pub hotspot: Hotspot,
}

impl ContentRow for CursorDef {
    const TABLE: &'static str = "cursor";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotspot {
    pub x: u16,
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

#[derive(Clone)]
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
}

impl Fixture for InterfaceIcon {
    type Row = InterfaceIconDef;
}

#[derive(Clone)]
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

pub fn font_files(content: &Content) -> Vec<String> {
    FontFace::VARIANTS
        .iter()
        .map(|face| face.get(content).file.0.to_owned())
        .collect()
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

pub fn check(assets: &AssetService) {
    let content = assets.content();
    let images = content
        .table::<CursorDef>()
        .rows()
        .iter()
        .map(|cursor| cursor.image)
        .chain(
            content
                .table::<InterfaceIconDef>()
                .rows()
                .iter()
                .map(|icon| icon.image),
        )
        .chain(
            content
                .table::<FontDef>()
                .rows()
                .iter()
                .map(|font| font.file),
        );
    for file in images {
        if let Err(error) = assets.open(std::path::Path::new(file.0)) {
            panic!("{}: {error}", file.0);
        }
    }
}
