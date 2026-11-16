//! How the interface names one of the things the game is made of, wherever it mentions one: the
//! thing's icon, then its name in its colour. Chat, the lines the game tells a player and every
//! window draw a mention from here, so a thing looks the same wherever it is named.

use bevy_ecs::prelude::*;
use ui::egui::{CursorIcon, Rect, Response, Sense, Ui};
use ui::ink::{Ground, Prose};

use crate::core::card::CardStyleDef;
use crate::core::content::{Content, Fixture, ImageRef};
use crate::core::math::Rgb;
use crate::core::text::{self, TextLook};
use crate::systems::area::AreaId;
use crate::systems::area_map::{self, MapSight};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::{self, ItemId};
use crate::systems::quest::{self, QuestId};
use crate::systems::skill::{self, SkillId};
use crate::systems::text::{LineText, NameMark, SpanText};

/// One of the things a line can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mention {
    Item(ItemId),
    Quest(QuestId),
    Skill(SkillId),
    Area(AreaId),
}

/// A thing as it shows where it is named.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownMention {
    pub name: String,
    pub icon: ImageRef,
    pub color: Rgb,
}

impl Mention {
    /// An item's colour is its rarity's, a quest's or a skill's that of the card it is shown on,
    /// and an area's the one prose names places in.
    pub fn shown(self, content: &Content) -> ShownMention {
        match self {
            Mention::Item(item) => {
                let def = item.get(content);
                ShownMention {
                    name: def.name.clone(),
                    icon: def.icon.clone(),
                    color: def.rarity.get(content).gradient.from,
                }
            }
            Mention::Quest(quest) => {
                let def = quest.get(content);
                ShownMention {
                    name: def.title.clone(),
                    icon: def.icon(content).clone(),
                    color: CardStyleDef::of(content, def.card_style).ground.from,
                }
            }
            Mention::Skill(skill) => {
                let def = skill.get(content);
                ShownMention {
                    name: def.name.clone(),
                    icon: def.icon.clone(),
                    color: CardStyleDef::of(content, def.card_style).ground.from,
                }
            }
            Mention::Area(area) => {
                let place = Prose::Place.on(Ground::Dusk);
                ShownMention {
                    name: area.get(content).name.clone(),
                    icon: InterfaceIcon::Map.get(content).image.clone(),
                    color: Rgb([place.r(), place.g(), place.b()]),
                }
            }
        }
    }

    /// The thing as a line of text names it.
    pub fn span(self, content: &Content) -> SpanText {
        let shown = self.shown(content);
        SpanText::Named {
            name: shown.name,
            mark: NameMark {
                icon: Some(shown.icon.path()),
                color: shown.color,
            },
        }
    }

    /// The thing's name in its colour, for a line that already stands behind the thing's icon.
    pub fn name(self, content: &Content) -> SpanText {
        let shown = self.shown(content);
        SpanText::Named {
            name: shown.name,
            mark: NameMark {
                icon: None,
                color: shown.color,
            },
        }
    }

    /// A line that names the thing and says nothing else.
    pub fn line(self, content: &Content) -> LineText {
        self.span(content).into()
    }

    /// Shows the player what the mention names: the card of an item, a quest or a skill, dealt
    /// from `from`, and an area on the map.
    pub fn open(self, world: &mut World, from: Option<Rect>) {
        match self {
            Mention::Item(item) => item::card::open(world, item, from),
            Mention::Quest(quest) => quest::card::open(world, quest, from),
            Mention::Skill(skill) => skill::card::open(world, skill, from),
            Mention::Area(area) => area_map::open_on(world, MapSight::Area(area)),
        }
    }
}

impl From<ItemId> for Mention {
    fn from(item: ItemId) -> Mention {
        Mention::Item(item)
    }
}

impl From<QuestId> for Mention {
    fn from(quest: QuestId) -> Mention {
        Mention::Quest(quest)
    }
}

impl From<SkillId> for Mention {
    fn from(skill: SkillId) -> Mention {
        Mention::Skill(skill)
    }
}

impl From<AreaId> for Mention {
    fn from(area: AreaId) -> Mention {
        Mention::Area(area)
    }
}

/// Draws `mention` in a window, and shows what it names to a player who clicks it.
pub fn show(
    ui: &mut Ui,
    world: &mut World,
    mention: impl Into<Mention>,
    look: TextLook,
) -> Response {
    let mention = mention.into();
    let line = mention.line(world.resource::<Content>()).styled();
    let drawn = text::show(ui, world, &line, look, None);
    let link = ui
        .interact(drawn.rect, drawn.id.with("mention"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if link.clicked() {
        mention.open(world, Some(link.rect));
    }
    link
}
