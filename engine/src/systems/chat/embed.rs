//! What a chat line holds in its text beside words: one of the things the game is made of, or
//! a spot in one of its areas, shown as the interface mentions such a thing anywhere.

use bevy_ecs::prelude::*;
use bevy_terminal::TerminalEmbed;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use super::ChatMessages;
use crate::core::content::{Content, ContentFile};
use crate::core::math::Pos;
use crate::core::text::LineText;
use crate::core::units::{MeterSize, Meters};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::item::{ItemDef, ItemId};
use crate::systems::mention::{Mention, ShownMention};
use crate::systems::messages;
use crate::systems::movement::position;
use crate::systems::quest::{QuestDef, QuestId};
use crate::systems::skill::{SkillDef, SkillId};

/// What a player types before a kind's name to ask for a thing of it.
pub const EMBED_ASK: char = '@';

/// The kinds of thing a line can hold, in the order chat lists them. A player asks for one by
/// its name.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    IntoStaticStr,
    EnumString,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChatEmbedKind {
    Item,
    Quest,
    Skill,
    Location,
}

impl ChatEmbedKind {
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// What the things of the kind are called together.
    pub fn plural(self, words: &ChatMessages) -> &str {
        match self {
            ChatEmbedKind::Item => words.items(),
            ChatEmbedKind::Quest => words.quests(),
            ChatEmbedKind::Skill => words.skills(),
            ChatEmbedKind::Location => words.locations(),
        }
    }

    /// What a thing of the kind is, in a word or two.
    pub fn hint(self, words: &ChatMessages) -> &str {
        match self {
            ChatEmbedKind::Item => words.item_hint(),
            ChatEmbedKind::Quest => words.quest_hint(),
            ChatEmbedKind::Skill => words.skill_hint(),
            ChatEmbedKind::Location => words.location_hint(),
        }
    }

    /// The table whose files are the things of the kind, which a search for one of them lists.
    /// A kind with no table has nothing to pick: asking for it sets it in the line at once.
    pub fn table(self) -> Option<&'static str> {
        match self {
            ChatEmbedKind::Item => Some(ItemDef::TABLE),
            ChatEmbedKind::Quest => Some(QuestDef::TABLE),
            ChatEmbedKind::Skill => Some(SkillDef::TABLE),
            ChatEmbedKind::Location => None,
        }
    }
}

/// One thing a chat line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatEmbed {
    Item(ItemId),
    Quest(QuestId),
    Skill(SkillId),
    Location(ChatLocation),
}

/// A spot in one of the game's areas, to the metre: where a speaker stands, or one they picked on
/// the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChatLocation {
    pub area: AreaId,
    pub x: i32,
    pub y: i32,
}

impl ChatLocation {
    /// Where `speaker` stands, when they stand in an area.
    pub fn of(world: &World, speaker: Entity) -> Option<ChatLocation> {
        let at = position(world, speaker)?;
        Some(ChatLocation {
            area: world.get::<AreaTag>(speaker)?.area,
            x: at.x.round() as i32,
            y: at.y.round() as i32,
        })
    }

    pub fn at(self) -> Pos<Meters> {
        Pos::new(self.x as f32, self.y as f32)
    }
}

/// A location as a line sent over a terminal keys it.
#[derive(Serialize, Deserialize)]
struct SentChatLocation {
    area: String,
    x: i32,
    y: i32,
}

impl ChatEmbed {
    pub fn kind(self) -> ChatEmbedKind {
        match self {
            ChatEmbed::Item(_) => ChatEmbedKind::Item,
            ChatEmbed::Quest(_) => ChatEmbedKind::Quest,
            ChatEmbed::Skill(_) => ChatEmbedKind::Skill,
            ChatEmbed::Location(_) => ChatEmbedKind::Location,
        }
    }

    /// The thing of `kind` whose file is at `path`, when the game has one.
    pub fn of(content: &Content, kind: ChatEmbedKind, path: &str) -> Option<ChatEmbed> {
        match kind {
            ChatEmbedKind::Item => Some(ChatEmbed::Item(content.by_path(path)?)),
            ChatEmbedKind::Quest => Some(ChatEmbed::Quest(content.by_path(path)?)),
            ChatEmbedKind::Skill => Some(ChatEmbed::Skill(content.by_path(path)?)),
            ChatEmbedKind::Location => None,
        }
    }

    /// What a line sent over a terminal holds, when it is a thing of the game or a spot in one
    /// of its areas.
    pub fn read(content: &Content, sent: &TerminalEmbed) -> Option<ChatEmbed> {
        match sent.kind.parse().ok()? {
            ChatEmbedKind::Location => {
                let sent: SentChatLocation = serde_json::from_str(&sent.key).ok()?;
                Some(ChatEmbed::Location(ChatLocation {
                    area: content.by_path(&sent.area)?,
                    x: sent.x,
                    y: sent.y,
                }))
            }
            kind => ChatEmbed::of(content, kind, &sent.key),
        }
    }

    /// The thing as a line sent over a terminal holds it.
    pub fn sent(self, content: &Content) -> TerminalEmbed {
        let key = match self {
            ChatEmbed::Item(item) => item.path(content).to_owned(),
            ChatEmbed::Quest(quest) => quest.path(content).to_owned(),
            ChatEmbed::Skill(skill) => skill.path(content).to_owned(),
            ChatEmbed::Location(location) => {
                let sent = SentChatLocation {
                    area: location.area.path(content).to_owned(),
                    x: location.x,
                    y: location.y,
                };
                serde_json::to_string(&sent).expect("a location writes as json")
            }
        };
        TerminalEmbed {
            kind: self.kind().name().to_owned(),
            key,
        }
    }

    /// What a line a player sends may hold of this: a location only on the ground of its area,
    /// and anything else as itself.
    pub fn held(self, content: &Content) -> Option<ChatEmbed> {
        match self {
            ChatEmbed::Location(location) => area::loaded(content, location.area)
                .is_some_and(|area| area.size.bounds().contains(location.at()))
                .then_some(self),
            thing => Some(thing),
        }
    }

    /// How it shows in a line: a thing as the interface mentions it anywhere, and a location
    /// as its area is mentioned, with the spot after the area's name.
    /// The thing as a line names it: an item as its inline item does.
    pub fn line(self, content: &Content) -> LineText {
        match self {
            ChatEmbed::Item(item) => Mention::Item(item).line(content),
            ChatEmbed::Quest(quest) => Mention::Quest(quest).line(content),
            ChatEmbed::Skill(skill) => Mention::Skill(skill).line(content),
            ChatEmbed::Location(_) => self.shown(content).span().into(),
        }
    }

    pub fn shown(self, content: &Content) -> ShownMention {
        match self {
            ChatEmbed::Item(item) => Mention::Item(item).shown(content),
            ChatEmbed::Quest(quest) => Mention::Quest(quest).shown(content),
            ChatEmbed::Skill(skill) => Mention::Skill(skill).shown(content),
            ChatEmbed::Location(location) => {
                let area = Mention::Area(location.area).shown(content);
                ShownMention {
                    name: messages::of(content)
                        .chat
                        .location_spot(&area.name, location.x, location.y),
                    ..area
                }
            }
        }
    }
}

/// What a player said on a channel: the words, and what they hold at each of the places the
/// words mark for a thing.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSaid {
    pub text: String,
    pub embeds: Vec<TerminalEmbed>,
}
