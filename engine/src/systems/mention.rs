//! How the interface names one of the things the game is made of, wherever it mentions one: the
//! thing's icon, then its name in its colour. Chat, the lines the game tells a player and every
//! window draw a mention from here, so a thing looks the same wherever it is named.

use bevy_ecs::prelude::*;
use ui::controls;
use ui::egui::{Rect, Response, Ui};
use ui::face::TextRole;
use ui::ink;
use ui::ink::{Ground, Prose};
use ui::suggest::DrawnSuggestion;

use crate::core::card::CardStyleDef;
use crate::core::content::{Content, Fixture, ImageRef};
use crate::core::math::Rgb;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::area::AreaId;
use crate::systems::area_map::{self, MapSight};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::inline::{self, InlineItem};
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

impl ShownMention {
    /// The thing as a line of text names it: its icon, then its name in its colour.
    pub fn span(self) -> SpanText {
        SpanText::Named {
            name: self.name,
            mark: NameMark {
                icon: Some(self.icon.path()),
                color: self.color,
            },
        }
    }
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

    /// The thing as a line of text names it: an item as its inline item does.
    pub fn span(self, content: &Content) -> SpanText {
        match self {
            Mention::Item(item) => InlineItem::of(item).into(),
            _ => self.shown(content).span(),
        }
    }

    /// The thing's name in its colour, for a line that already stands behind the thing's icon.
    /// An item is named only as its inline item names it, icon and all.
    pub fn name(self, content: &Content) -> SpanText {
        if let Mention::Item(item) = self {
            return InlineItem::of(item).into();
        }
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

/// A dropdown that picks one of `choices` into `picked`, the one picked shown on it and every
/// one in the list it opens drawn as `mention` draws it. An item shows its tooltip there, the
/// dropdown taking the click.
pub fn dropdown<T: Into<Mention> + Copy + PartialEq>(
    ui: &mut Ui,
    world: &mut World,
    id_salt: impl ui::egui::AsIdSalt,
    picked: &mut T,
    choices: &[T],
) -> Response {
    let listed: Vec<(Option<&str>, T, Mention)> = choices
        .iter()
        .map(|&choice| (None, choice, choice.into()))
        .collect();
    pick(ui, world, id_salt, picked, &listed)
}

/// A dropdown that picks into `picked` one of the choices of `groups`, each a run of the list
/// under its heading, and shows the one picked after its group's heading.
pub fn grouped_dropdown<T: Copy + PartialEq>(
    ui: &mut Ui,
    world: &mut World,
    id_salt: impl ui::egui::AsIdSalt,
    picked: &mut T,
    groups: &[(&str, Vec<(T, Mention)>)],
) -> Response {
    let listed: Vec<(Option<&str>, T, Mention)> = groups
        .iter()
        .flat_map(|(heading, choices)| {
            choices
                .iter()
                .map(move |&(choice, mention)| (Some(*heading), choice, mention))
        })
        .collect();
    pick(ui, world, id_salt, picked, &listed)
}

/// What a mention dropdown shows of each of its choices: in words, on its button and in its list,
/// and the heading of the group it starts, where it starts one.
struct MentionChoices<'w> {
    world: &'w World,
    names: Vec<String>,
    buttons: Vec<StyledLine>,
    rows: Vec<StyledLine>,
    headings: Vec<Option<String>>,
}

impl controls::DrawnChoices for MentionChoices<'_> {
    fn count(&self) -> usize {
        self.rows.len()
    }

    fn name(&self, place: usize) -> String {
        self.names.get(place).cloned().unwrap_or_default()
    }

    fn heading(&self, place: usize) -> Option<String> {
        self.headings.get(place).cloned().flatten()
    }

    fn widest(&self, ui: &Ui, part: controls::ComboPart) -> f32 {
        self.lines(part)
            .iter()
            .map(|line| text::measure(ui.ctx(), self.world, line, dropdown_look(), f32::INFINITY).x)
            .fold(0.0, f32::max)
    }

    fn draw(&self, ui: &mut Ui, place: usize, part: controls::ComboPart) {
        if let Some(line) = self.lines(part).get(place) {
            text::show(ui, self.world, line, dropdown_look(), None);
        }
    }
}

impl MentionChoices<'_> {
    fn lines(&self, part: controls::ComboPart) -> &[StyledLine] {
        match part {
            controls::ComboPart::Button => &self.buttons,
            controls::ComboPart::List => &self.rows,
        }
    }
}

fn pick<T: Copy + PartialEq>(
    ui: &mut Ui,
    world: &mut World,
    id_salt: impl ui::egui::AsIdSalt,
    picked: &mut T,
    listed: &[(Option<&str>, T, Mention)],
) -> Response {
    let content = world.resource::<Content>().clone();
    let world_ref: &World = world;
    let mut choices = MentionChoices {
        world: world_ref,
        names: Vec::new(),
        buttons: Vec::new(),
        rows: Vec::new(),
        headings: Vec::new(),
    };
    let mut group = None;
    for &(heading, _, mention) in listed {
        let name = mention.shown(&content).name;
        let row = mention.line(&content);
        let (name, button) = match heading {
            Some(heading) => (
                format!("{heading}: {name}"),
                LineText::plain(format!("{heading}: ")).then(row.clone()),
            ),
            None => (name, row.clone()),
        };
        choices.names.push(name);
        choices.buttons.push(button.styled(&content));
        choices.rows.push(row.styled(&content));
        choices.headings.push(
            heading
                .filter(|_| group != Some(heading))
                .map(str::to_owned),
        );
        group = Some(heading);
    }
    let selected = listed
        .iter()
        .position(|(_, choice, _)| choice == picked)
        .unwrap_or(0);
    let combo = controls::drawn_combo(ui, id_salt, &choices, selected);
    let pointed = combo
        .pointed
        .map(|(place, row)| (row, place))
        .into_iter()
        .chain([(combo.response.clone(), selected)]);
    for (response, place) in pointed {
        if let Some((_, _, Mention::Item(item))) = listed.get(place) {
            inline::tooltip(world, &response, InlineItem::of(*item));
        }
    }
    if let Some((_, choice, _)) = combo.picked.and_then(|place| listed.get(place)) {
        *picked = *choice;
    }
    combo.response
}

fn dropdown_look() -> TextLook {
    TextLook::new(TextRole::Button.size(), ink::INK)
}

/// A thing listed among the suggestions for what is being typed, drawn as `show` draws it.
pub struct SuggestedMention<'w> {
    world: &'w World,
    line: StyledLine,
    width: f32,
}

impl<'w> SuggestedMention<'w> {
    pub fn new(world: &'w World, gui: &ui::egui::Context, line: &LineText) -> Self {
        let line = line.styled(world.resource::<Content>());
        let width = text::measure(gui, world, &line, suggested_look(), f32::INFINITY).x;
        SuggestedMention { world, line, width }
    }
}

impl DrawnSuggestion for SuggestedMention<'_> {
    fn width(&self) -> f32 {
        self.width
    }

    fn draw(&self, ui: &mut Ui) {
        text::show(ui, self.world, &self.line, suggested_look(), None);
    }
}

fn suggested_look() -> TextLook {
    TextLook::new(TextRole::Tooltip.size(), ink::INK)
}
