use bevy_ecs::world::World;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::ImageRef;
use crate::core::audio::playback::{PlaySfx, SfxDef, SfxPlace};
use crate::core::content::{Content, ContentFile, Fixture, Nearby};

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct CursorDef {
    pub image: ImageRef,
    pub hotspot: Hotspot,
}

impl ContentFile for CursorDef {
    const TABLE: &'static str = "cursor";

    fn fresh(near: &Nearby) -> Result<CursorDef, String> {
        Ok(CursorDef {
            image: near.image()?,
            hotspot: Hotspot { x: 0, y: 0 },
        })
    }
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
    Enter,
    Attack,
    PickUp,
    Talk,
    Use,
    Open,
    Read,
    Target,
}

impl Fixture for CursorShape {
    type Row = CursorDef;
    const FOLDER: Option<&'static str> = Some("interface");
    const WHOLE_TABLE: bool = true;
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InterfaceIconDef {
    pub image: ImageRef,
}

impl ContentFile for InterfaceIconDef {
    const TABLE: &'static str = "interface_icon";

    fn fresh(near: &Nearby) -> Result<InterfaceIconDef, String> {
        Ok(InterfaceIconDef {
            image: near.image()?,
        })
    }
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
    Terminal,
    Skills,
}

impl Fixture for InterfaceIcon {
    type Row = InterfaceIconDef;
    const FOLDER: Option<&'static str> = Some("interface");
    const WHOLE_TABLE: bool = true;
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
    const FOLDER: Option<&'static str> = Some("interface");
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
