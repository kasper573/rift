use bevy_ecs::world::World;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::playback::{PlaySfx, SfxDef, SfxPlace};
use crate::core::content::ImageRef;
use crate::core::content::SchemaMark;
use crate::core::content::{Content, ContentFile, ContentId, Fixture, Nearby};
use crate::core::text::{LineText, SpanText};
use crate::systems::messages::messages;

pub type CursorId = ContentId<CursorDef>;

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
    #[schemars(transform = SchemaMark::Unit.with("px"))]
    pub x: u16,
    #[schemars(transform = SchemaMark::Unit.with("px"))]
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
    Character,
    Skills,
    Quests,
    Map,
    History,
    Social,
    Settings,
    Mail,
}

impl Fixture for InterfaceIcon {
    type Row = InterfaceIconDef;
    const FOLDER: Option<&'static str> = Some("interface");
    const WHOLE_TABLE: bool = true;
}

messages! {
    /// What the game says on the parts of its interface every window shares.
    pub struct InterfaceMessages {
        /// The button that puts away what the player was told.
        ok,
        /// The button that backs out of what the player was asked.
        cancel,
        /// What closes a window, as a screen reader names its cross.
        close,
        /// What the corner that resizes a window is named by a screen reader.
        resize,
        /// The button that takes a row out of a list the player is making.
        remove,
        /// The button that turns to the page before.
        back,
        /// The button that turns to the page after.
        next,
        /// The heading over the player's inventory, where a window shows it.
        inventory,
        /// What a screen reader names a choice listed under a heading, `{group}` being the heading and `{choice}` the choice's name.
        grouped_choice(group: Text, choice: Text),
        /// What a dropdown's button shows of a choice listed under a heading, `{group}` being the heading and `{choice}` the choice.
        grouped_choice_line[group: Text, choice: Line],
        /// A list of two, `{first}` and `{second}`, as CLDR's list pattern for two words it.
        list_pair[first: Line, second: Line],
        /// The start of a longer list: its first item, `{first}`, before the rest, `{rest}`.
        list_start[first: Line, rest: Line],
        /// The middle of a longer list: an item, `{first}`, before the rest, `{rest}`.
        list_middle[first: Line, rest: Line],
        /// The end of a longer list: the item before the last, `{first}`, and the last, `{last}`.
        list_end[first: Line, last: Line],
    }
}

impl InterfaceMessages {
    /// The `lines` as one, the way a sentence lists them in the catalog's language.
    pub fn listed(&self, mut lines: Vec<LineText>) -> LineText {
        let Some(last) = lines.pop() else {
            return LineText::default();
        };
        let Some(before) = lines.pop() else {
            return last;
        };
        if lines.is_empty() {
            return self.list_pair(before, last);
        }
        let mut rest = self.list_end(before, last);
        while lines.len() > 1 {
            let item = lines.pop().unwrap_or_default();
            rest = self.list_middle(item, rest);
        }
        let first = lines.pop().unwrap_or_default();
        self.list_start(first, rest)
    }

    /// The `words` as one, the way a sentence lists them in the catalog's language.
    pub fn listed_words(&self, words: Vec<String>) -> String {
        self.listed(words.into_iter().map(LineText::plain).collect())
            .0
            .into_iter()
            .filter_map(|span| match span {
                SpanText::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }
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
