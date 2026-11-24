//! The inline item: an item named wherever it isn't a tile, in a line of text, a row, a price or
//! a footer. It shows the item's icon and its name in its rarity's colour, and how many of it
//! there are where an amount is given. A view may leave the name out where its players know the
//! item by its icon alone; the amount then stands after the icon rather than before it. Pointing
//! at it shows the item's tooltip, and clicking it opens the item's card.
//!
//! It is drawn two ways that look the same: inside a line of text by the line's pieces, and by
//! `draw` and `draw_line`, which need nothing but the icons, so the editor's previews draw it too.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Color32, CursorIcon, Id, Response, Sense, Ui, Vec2};
use ui::face::TextRole;
use ui::ink::Ground;

use super::{ItemId, ItemStack, card};
use crate::core::content::Content;
use crate::core::gui::ImageSource;
use crate::core::text::{
    self, KeyNames, LineText, LinkKey, PluggedSpan, ShownLine, SpanDef, SpanKinds, SpanText,
    StyledLine, StyledSpan, TextLook, TextPiece,
};
use crate::systems::input::map::{self, InputAction};
use crate::systems::messages::{self, Line, Number};

/// An item as a line names it, and how it shows there.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InlineItem {
    pub item: ItemId,
    pub amount: Option<ItemAmount>,
    pub shows: ItemShows,
}

/// What of an inline item shows, in the order it reads.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemShows {
    /// How many, its icon and its name.
    Named,
    /// Its icon and how many, for a view whose players know the item by its icon.
    Icon,
    /// How many and then its icon, as a tally counts what is paid or got.
    Tallied,
}

/// How many of an item an inline item says there are: a whole count, or a share of one, as a
/// price for one item is.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ItemAmount {
    Whole(u32),
    Share(f32),
}

impl From<ItemAmount> for Number {
    fn from(amount: ItemAmount) -> Number {
        match amount {
            ItemAmount::Whole(count) => count.into(),
            ItemAmount::Share(share) => share.into(),
        }
    }
}

impl InlineItem {
    /// The item by its icon and name.
    pub fn of(item: ItemId) -> InlineItem {
        InlineItem {
            item,
            amount: None,
            shows: ItemShows::Named,
        }
    }

    /// With how many of the item there are.
    pub fn count(self, count: u32) -> InlineItem {
        InlineItem {
            amount: Some(ItemAmount::Whole(count)),
            ..self
        }
    }

    /// With the share of one item there is.
    pub fn share(self, share: f32) -> InlineItem {
        InlineItem {
            amount: Some(ItemAmount::Share(share)),
            ..self
        }
    }

    /// By its icon alone, for a view whose players know the item by it.
    pub fn nameless(self) -> InlineItem {
        InlineItem {
            shows: ItemShows::Icon,
            ..self
        }
    }

    /// By how many and its icon, as a tally counts it.
    pub fn tallied(self) -> InlineItem {
        InlineItem {
            shows: ItemShows::Tallied,
            ..self
        }
    }

    /// Whether its name shows beside its icon.
    pub fn named(self) -> bool {
        self.shows == ItemShows::Named
    }

    /// Its parts in the order they read, the same for a line and for `draw`.
    fn parts(self) -> Vec<Part> {
        let amount = self.amount.map(Part::Amount);
        match self.shows {
            ItemShows::Named => amount.into_iter().chain([Part::Icon, Part::Name]).collect(),
            ItemShows::Icon => std::iter::once(Part::Icon).chain(amount).collect(),
            ItemShows::Tallied => amount.into_iter().chain([Part::Icon]).collect(),
        }
    }
}

impl SpanDef for InlineItem {
    fn pieces(&self, content: &Content) -> Vec<TextPiece> {
        let def = self.item.get(content);
        let color = Color32::from(def.rarity.get(content).gradient.from);
        let shown = self.parts().into_iter().map(|part| match part {
            Part::Amount(amount) if self.shows != ItemShows::Icon => {
                TextPiece::text(format!("{} ", messages::of(content).item.amount(amount)))
            }
            Part::Amount(amount) => TextPiece::text(messages::of(content).item.amount(amount)),
            Part::Icon => TextPiece::Image(def.icon.path().into()),
            Part::Name => StyledSpan::plain(def.name.clone()).legible(color).into(),
        });
        let link = ItemLink {
            item: self.item,
            named: self.named(),
        };
        let name = self.words(content, None);
        std::iter::once(TextPiece::LinkStart {
            key: link.key(),
            name,
        })
        .chain(shown)
        .chain([TextPiece::LinkEnd])
        .collect()
    }

    /// How many, and the item's name, whether it shows or not.
    fn words(&self, content: &Content, _: Option<&KeyNames>) -> String {
        let name = &self.item.get(content).name;
        match self.amount {
            Some(amount) => messages::of(content).item.counted(amount, name),
            None => name.clone(),
        }
    }
}

impl From<ItemStack> for InlineItem {
    fn from(stack: ItemStack) -> InlineItem {
        InlineItem::of(stack.item).count(stack.count)
    }
}

impl From<InlineItem> for SpanText {
    fn from(item: InlineItem) -> SpanText {
        SpanText::Part(PluggedSpan::new(item))
    }
}

impl From<InlineItem> for Line {
    fn from(item: InlineItem) -> Line {
        LineText::from(item).into()
    }
}

impl From<InlineItem> for LineText {
    fn from(item: InlineItem) -> LineText {
        SpanText::from(item).into()
    }
}

/// Registers the inline item as a kind of span.
pub fn spans(kinds: &mut SpanKinds) {
    kinds.add::<InlineItem>("item");
}

/// How an inline item standing on its own is lettered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineLook {
    pub role: TextRole,
    /// The ink of its amount.
    pub color: Color32,
    pub ground: Ground,
}

impl InlineLook {
    pub const fn new(role: TextRole, color: Color32) -> InlineLook {
        InlineLook {
            role,
            color,
            ground: Ground::Paper,
        }
    }

    pub const fn on(self, ground: Ground) -> InlineLook {
        InlineLook { ground, ..self }
    }
}

/// Draws `item` on its own, its icon from `images`, sensing clicks: what it does to a pointer is
/// for `inspect` or `tooltip` to say, wherever the game is at hand.
pub fn draw(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    item: InlineItem,
    look: InlineLook,
) -> Response {
    let def = item.item.get(content);
    let size = look.role.size();
    let shown = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = size * 0.25;
            for part in item.parts() {
                match part {
                    Part::Amount(amount) => {
                        let said = messages::of(content).item.amount(amount);
                        ui.label(look.role.rich(said).color(look.color));
                    }
                    Part::Icon => match images.image(&def.icon) {
                        Some(icon) => {
                            ui.add(icon.widget(Vec2::splat(size)));
                        }
                        None => {
                            ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                        }
                    },
                    Part::Name => {
                        let rarity = Color32::from(def.rarity.get(content).gradient.from);
                        let name = look.role.rich(def.name.as_str());
                        let name = name.color(look.ground.legible(rarity));
                        ui.add(
                            egui::Label::new(name)
                                .truncate()
                                .show_tooltip_when_elided(false),
                        );
                    }
                }
            }
        })
        .response;
    let response = ui.interact(shown.rect, shown.id.with("inline item"), Sense::click());
    let words = item.words(content, None);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &words));
    response
}

/// Draws `line` where the game isn't at hand, as `draw` draws an item: its words, wrapped, and its
/// items as inline items. Answers each item drawn, for `inspect` to have answer the pointer.
pub fn draw_line(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    line: &LineText,
    look: InlineLook,
) -> Vec<(Response, InlineItem)> {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut drawn = Vec::new();
        for span in &line.0 {
            let item = match span {
                SpanText::Part(part) => part.downcast_ref::<InlineItem>().copied(),
                _ => None,
            };
            match item {
                Some(item) => {
                    let shown = ui.scope(|ui| draw(ui, images, content, item, look)).inner;
                    drawn.push((shown, item));
                }
                None => {
                    let words = LineText(vec![span.clone()]).plain_words(content);
                    ui.label(look.role.rich(words).color(look.color));
                }
            }
        }
        drawn
    })
    .inner
}

/// What a drawn inline item does to a pointer: shows its item's tooltip, and opens the item's
/// card to a click.
pub fn inspect(world: &mut World, response: Response, item: InlineItem) -> Response {
    inspect_item(world, &response, item.item, item.named());
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// What a drawn inline item does to a pointer where a click on it belongs elsewhere: in a row
/// clicked as a whole, which shows what it stands for, in a dialog the card would open behind, or
/// in a text field, which puts its caret there. It shows its item's tooltip, and leaves the click
/// alone.
pub fn tooltip(world: &mut World, response: &Response, item: InlineItem) {
    item_tooltip(world, response, item.item, item.named(), None);
}

/// Draws `item` on its own, as `draw` does, showing its tooltip and opening its card.
pub fn show(ui: &mut Ui, world: &mut World, item: InlineItem, look: InlineLook) -> Response {
    let content = world.resource::<Content>().clone();
    let response = draw(ui, world, &content, item, look);
    inspect(world, response, item)
}

/// Draws `item` where a click on it belongs elsewhere, as `tooltip` has it answer.
pub fn show_with_tooltip(ui: &mut Ui, world: &mut World, item: InlineItem, look: InlineLook) {
    let content = world.resource::<Content>().clone();
    let response = draw(ui, world, &content, item, look);
    tooltip(world, &response, item);
}

/// Draws `line`, every inline item in it showing its item's tooltip to a pointer over it and
/// opening its card to a click, as far as `shown` reveals the line.
pub fn show_line(
    ui: &mut Ui,
    world: &mut World,
    line: &LineText,
    look: TextLook,
    shown: Option<usize>,
) -> Response {
    let styled = line.styled(world.resource::<Content>());
    show_styled(ui, world, &styled, look, shown)
}

/// Draws `line` with its inline items answering nothing, where the line stands in a row whose
/// own tile carries the item's tooltip.
pub fn show_plain_line(
    ui: &mut Ui,
    world: &mut World,
    line: &LineText,
    look: TextLook,
) -> Response {
    let styled = line.styled(world.resource::<Content>());
    text::show(ui, world, &styled, look, None)
}

/// Draws `line` where a click on it belongs elsewhere, as `tooltip` has its inline items answer.
pub fn show_line_with_tooltips(
    ui: &mut Ui,
    world: &mut World,
    line: &LineText,
    look: TextLook,
) -> Response {
    let styled = line.styled(world.resource::<Content>());
    draw_styled(ui, world, &styled, look, None, Clicks::Elsewhere)
}

/// Draws a line already styled, as `show_line` does.
pub fn show_styled(
    ui: &mut Ui,
    world: &mut World,
    line: &StyledLine,
    look: TextLook,
    shown: Option<usize>,
) -> Response {
    draw_styled(ui, world, line, look, shown, Clicks::Item)
}

/// Has every inline item of the line `drawn` show its item's tooltip to a pointer over it and
/// open its card to a click, for a drawer that draws a line itself to answer links of its own.
pub fn answer_items(ui: &mut Ui, world: &mut World, drawn: &ShownLine) {
    answer(ui, world, drawn, Clicks::Item);
}

/// A part of an inline item.
enum Part {
    Amount(ItemAmount),
    Icon,
    Name,
}

/// What a line's link to an inline item holds as its key: the item, and whether its name shows
/// beside it, which says whether its tooltip names it.
struct ItemLink {
    item: ItemId,
    named: bool,
}

impl ItemLink {
    fn key(self) -> LinkKey {
        LinkKey::new::<ItemLink>(u64::from(self.item.raw()) << 1 | u64::from(self.named))
    }

    fn of(key: LinkKey) -> Option<ItemLink> {
        let id = key.id_in::<ItemLink>()?;
        Some(ItemLink {
            item: ItemId::new((id >> 1) as u32),
            named: id & 1 == 1,
        })
    }
}

/// Whose a click on an inline item is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Clicks {
    Item,
    Elsewhere,
}

/// Opens `item`'s card to a click on `response`, adds the item to chat when asked there, and
/// shows the item's tooltip.
fn inspect_item(world: &mut World, response: &Response, item: ItemId, named: bool) {
    if map::clicked(world, InputAction::InspectItem, response) {
        card::open(world, item, Some(response.rect));
    }
    card::chat_shortcut(world, response, &[item]);
    let hint = card::inspect_hint(world);
    item_tooltip(world, response, item, named, Some(&hint));
}

/// The item's tooltip, naming the item only where its name isn't shown beside it already.
fn item_tooltip(
    world: &mut World,
    response: &Response,
    item: ItemId,
    named: bool,
    hint: Option<&StyledLine>,
) {
    if named {
        card::tooltip_beside_name(world, response, item, [], hint);
    } else {
        card::tooltip_noted(world, response, item, [], hint);
    }
}

fn draw_styled(
    ui: &mut Ui,
    world: &mut World,
    line: &StyledLine,
    look: TextLook,
    shown: Option<usize>,
    clicks: Clicks,
) -> Response {
    let drawn = text::show_linked(ui, world, line, look, shown);
    answer(ui, world, &drawn, clicks);
    drawn.response
}

fn answer(ui: &mut Ui, world: &mut World, drawn: &ShownLine, clicks: Clicks) {
    for (index, shown) in drawn.links.iter().enumerate() {
        let Some(link) = ItemLink::of(shown.key) else {
            continue;
        };
        let (item, rect, name) = (link.item, &shown.row, &shown.name);
        let id = Id::new((drawn.response.id, "inline item", index));
        match clicks {
            Clicks::Item => {
                let response = ui
                    .interact(*rect, id, Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                });
                inspect_item(world, &response, item, link.named);
            }
            Clicks::Elsewhere => {
                let response = ui.interact(*rect, id, Sense::hover());
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name));
                item_tooltip(world, &response, item, link.named, None);
            }
        }
    }
}
