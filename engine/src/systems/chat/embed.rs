//! What a chat line holds in its text beside words: one of the things the game is made of, shown
//! by its icon and by its name in its colour.

use bevy_terminal::TerminalEmbed;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::core::card::CardStyleDef;
use crate::core::content::{Content, ContentFile, ImageRef};
use crate::core::math::Rgb;
use crate::systems::item::{ItemDef, ItemId};
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
}

impl ChatEmbedKind {
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// What the things of the kind are called together.
    pub fn plural(self) -> &'static str {
        match self {
            ChatEmbedKind::Item => "items",
            ChatEmbedKind::Quest => "quests",
            ChatEmbedKind::Skill => "skills",
        }
    }

    /// What a thing of the kind is, in a word or two.
    pub fn hint(self) -> &'static str {
        match self {
            ChatEmbedKind::Item => "an item",
            ChatEmbedKind::Quest => "a quest",
            ChatEmbedKind::Skill => "a skill",
        }
    }

    /// The table whose files are the things of the kind: what a search for one of them lists.
    pub fn table(self) -> &'static str {
        match self {
            ChatEmbedKind::Item => ItemDef::TABLE,
            ChatEmbedKind::Quest => QuestDef::TABLE,
            ChatEmbedKind::Skill => SkillDef::TABLE,
        }
    }
}

/// One thing a chat line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatEmbed {
    Item(ItemId),
    Quest(QuestId),
    Skill(SkillId),
}

impl ChatEmbed {
    pub fn kind(self) -> ChatEmbedKind {
        match self {
            ChatEmbed::Item(_) => ChatEmbedKind::Item,
            ChatEmbed::Quest(_) => ChatEmbedKind::Quest,
            ChatEmbed::Skill(_) => ChatEmbedKind::Skill,
        }
    }

    /// The thing of `kind` whose file is at `path`, when the game has one.
    pub fn of(content: &Content, kind: ChatEmbedKind, path: &str) -> Option<ChatEmbed> {
        Some(match kind {
            ChatEmbedKind::Item => ChatEmbed::Item(content.by_path(path)?),
            ChatEmbedKind::Quest => ChatEmbed::Quest(content.by_path(path)?),
            ChatEmbedKind::Skill => ChatEmbed::Skill(content.by_path(path)?),
        })
    }

    /// What a line sent over a terminal holds, when it is a thing of the game.
    pub fn read(content: &Content, sent: &TerminalEmbed) -> Option<ChatEmbed> {
        ChatEmbed::of(content, sent.kind.parse().ok()?, &sent.key)
    }

    /// The thing as a line sent over a terminal holds it.
    pub fn sent(self, content: &Content) -> TerminalEmbed {
        let path = match self {
            ChatEmbed::Item(item) => item.path(content),
            ChatEmbed::Quest(quest) => quest.path(content),
            ChatEmbed::Skill(skill) => skill.path(content),
        };
        TerminalEmbed {
            kind: self.kind().name().to_owned(),
            key: path.to_owned(),
        }
    }

    /// How the thing shows in a line. An item's colour is its rarity's, and a quest's or a
    /// skill's that of the card it is shown on.
    pub fn shown(self, content: &Content) -> ShownChatEmbed {
        match self {
            ChatEmbed::Item(item) => {
                let def = item.get(content);
                ShownChatEmbed {
                    name: def.name.clone(),
                    icon: def.icon.clone(),
                    color: def.rarity.get(content).gradient.from,
                }
            }
            ChatEmbed::Quest(quest) => {
                let def = quest.get(content);
                ShownChatEmbed {
                    name: def.title.clone(),
                    icon: def.icon(content).clone(),
                    color: CardStyleDef::of(content, def.card_style).ground.from,
                }
            }
            ChatEmbed::Skill(skill) => {
                let def = skill.get(content);
                ShownChatEmbed {
                    name: def.name.clone(),
                    icon: def.icon.clone(),
                    color: CardStyleDef::of(content, def.card_style).ground.from,
                }
            }
        }
    }
}

/// A thing a line holds as the line shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownChatEmbed {
    pub name: String,
    pub icon: ImageRef,
    pub color: Rgb,
}

/// What a player said on a channel: the words, and what they hold at each of the places the
/// words mark for a thing.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSaid {
    pub text: String,
    pub embeds: Vec<TerminalEmbed>,
}
