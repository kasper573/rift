use bevy::ecs::message::Messages;
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::card::{self, Card, CardResponse, CardTilt, Plate};
use ui::controls::{self, Pill};
use ui::drag;
use ui::egui;
use ui::egui::{Rect, Response, Ui};
use ui::face::{self, TextRole};
use ui::glyph::{self, CardKind, Mark};
use ui::ink;
use ui::sheet::Depth;

use crate::core::card::CardStyleDef;
use crate::core::content::Content;
use crate::core::gui::{self, Band, ImageSource};
use crate::core::math::ColorGradient;
use crate::core::script::{Expr, condition};
use crate::core::stage::card::HungCardLamp;
use crate::core::text::LineText;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::chat::ChatEmbed;
use crate::systems::hud::{self, Carried, WindowKey};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::ItemMessages;
use crate::systems::item::inline;
use crate::systems::item::{ItemDef, ItemId, ItemKind, ItemModuleKind, ItemShown, Wear};
use crate::systems::mention::Mention;
use crate::systems::messages::Direction;
use crate::systems::player::session;
use crate::systems::requirement;
use crate::systems::screen::ClientScreen;
use crate::systems::skill::{self, SkillDef, SkillId};
use crate::systems::stat::StatKind;
use crate::systems::status;
use crate::systems::terminal::chat;
use crate::systems::{GameSettings, messages};

pub struct ItemCardPlugin;

impl Plugin for ItemCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<hud::Pinned<ItemId>>()
            .add_systems(
                Update,
                (show_told, hud::raise_pinned::<ItemId>(WindowKey::ItemCard))
                    .chain()
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                draw_card
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), hud::forget_pinned::<ItemId>);
    }
}

fn show_told(world: &mut World) {
    let told: Vec<ItemShown> = world
        .resource_mut::<Messages<ItemShown>>()
        .drain()
        .collect();
    for ItemShown { item } in told {
        open(world, item, None);
    }
}

pub fn open(world: &mut World, item: ItemId, from: Option<Rect>) {
    hud::pin(world, item, from);
}

pub fn close(world: &mut World) -> bool {
    hud::unpin::<ItemId>(world)
}

pub fn showing(world: &World) -> bool {
    hud::pinned::<ItemId>(world).is_some()
}

pub fn layer() -> egui::LayerId {
    hud::window_layer(WindowKey::ItemCard)
}

/// How far the pinned card is turned toward the pointer.
pub fn tilt(world: &World) -> CardTilt {
    hud::pinned_tilt(world, WindowKey::ItemCard)
}

/// The lamp the pinned card is lit by, while it is on the stage.
pub fn lamp(world: &World) -> Option<HungCardLamp> {
    hud::pinned_lamp(world, WindowKey::ItemCard)
}

pub fn shown(world: &World) -> Option<(ItemId, Rect)> {
    Some((
        hud::pinned::<ItemId>(world)?,
        hud::pinned_rect::<ItemId>(world)?,
    ))
}

/// `response` must sense clicks.
pub fn inspect(world: &mut World, response: Response, item: ItemId) -> Response {
    if map::clicked(world, InputAction::InspectItem, &response) {
        open(world, item, Some(response.rect));
    }
    chat_shortcut(world, &response, &[item]);
    tooltip(world, &response, item);
    response
}

/// Sets the items `response` shows in chat once the player asks for it there, wherever items
/// are drawn. `response` must sense clicks.
pub fn chat_shortcut(world: &mut World, response: &Response, items: &[ItemId]) {
    chat::shortcut(world, response, items.iter().copied().map(ChatEmbed::Item));
}

pub fn tooltip(world: &mut World, response: &Response, item: ItemId) {
    let hint = inspect_hint(world);
    tooltip_noted(world, response, item, [], Some(&hint));
}

/// An item's tooltip: its name, a line led by its rarity that says what it is, `notes` under
/// that, and a hint.
pub fn tooltip_noted(
    world: &mut World,
    response: &Response,
    item: ItemId,
    notes: impl IntoIterator<Item = LineText>,
    hint: Option<&StyledLine>,
) {
    let name = item.get(world.resource::<Content>()).name.clone();
    tooltip_titled(world, response, item, Some(&name), notes, hint);
}

/// An item's tooltip beside where its name is shown already, as a named inline item shows it:
/// what the item is, `notes` under that, and a hint, without the name again.
pub fn tooltip_beside_name(
    world: &mut World,
    response: &Response,
    item: ItemId,
    notes: impl IntoIterator<Item = LineText>,
    hint: Option<&StyledLine>,
) {
    tooltip_titled(world, response, item, None, notes, hint);
}

fn tooltip_titled(
    world: &mut World,
    response: &Response,
    item: ItemId,
    title: Option<&str>,
    notes: impl IntoIterator<Item = LineText>,
    hint: Option<&StyledLine>,
) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let rarity = def.rarity.get(&content);
    let mark = rarity
        .icon
        .as_ref()
        .and_then(|icon| gui::image(world, icon));
    let messages = &messages::of(&content).item;
    let lines: Vec<LineText> =
        std::iter::once(format!("{} · {}", rarity.name, overview(def, messages)).into())
            .chain(notes)
            .collect();
    hud::tooltip_marked(response, world, title, mark.as_ref(), &lines, hint);
}

/// What an item is, in a line, for a tooltip, which has no band or style to say it: led by the
/// rarity's name there. A card says each part of it once elsewhere.
fn overview(def: &ItemDef, messages: &ItemMessages) -> String {
    let category = def.category().named(messages);
    match &def.kind {
        ItemKind::Equipment { wear, .. } => format!("{category} · {}", wear.named(messages)),
        _ => category,
    }
}

fn worn_as(wear: &Wear, messages: &ItemMessages) -> String {
    match wear {
        Wear::Held { .. } => messages.held_in_hand().to_owned(),
        Wear::Tool { .. } => messages.kept_as_tool().to_owned(),
        Wear::Worn { slot, .. } => messages.worn_on(slot.as_str().replace('_', " ")),
    }
}

/// How an item's card opens, and how it goes into chat where chat takes it, each input with what
/// it does.
pub fn inspect_hints(world: &World) -> Vec<(InputAction, String)> {
    let messages = &messages::of(world.resource::<Content>()).item;
    std::iter::once((InputAction::InspectItem, messages.inspect().to_owned()))
        .chain(chat_hint(world, messages))
        .collect()
}

/// How an item goes into chat, while chat takes it.
pub fn chat_hint(world: &World, messages: &ItemMessages) -> Option<(InputAction, String)> {
    chat::takes_embeds(world).then(|| (InputAction::AddToChat, messages.add_to_chat().to_owned()))
}

/// How an item's card opens and how it goes into chat, as a hint under it.
pub fn inspect_hint(world: &World) -> StyledLine {
    StyledLine::new(map::hints(inspect_hints(world)))
}

/// What using the item from the bag does, as a hint names it: none for an item that is not used.
pub fn use_hint<'w>(def: &ItemDef, messages: &'w ItemMessages) -> Option<&'w str> {
    match def.kind {
        ItemKind::Usable { .. } => Some(messages.use_item()),
        ItemKind::Equipment { .. } => Some(messages.wear()),
        ItemKind::Resource => None,
    }
}

pub fn kind(def: &ItemDef) -> CardKind {
    match def.kind {
        ItemKind::Equipment { .. } => CardKind::Equipment,
        ItemKind::Usable { .. } => CardKind::Usable,
        ItemKind::Resource => CardKind::Resource,
    }
}

/// An item's card, the same wherever it shows: a view that has more to say of the item says it
/// beside the card, never on it.
pub fn show(ui: &mut Ui, world: &mut World, item: ItemId, look: CardLook) -> CardResponse {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let held = super::my_bag(world).map_or(0, |bag| bag.count(item));
    let messages = messages::of(&content);
    let tally = (def.stack_max() > 1 && held > 0).then(|| messages.item.held(held));
    let band = def.category().named(&messages.item);
    let mut card = face_of(&content, world, item, look, &band);
    card.tally = tally.as_deref();
    card.action = Some(chat::card_action(&messages.terminal));
    let shown = card.show(ui, |ui, plate| {
        summary(ui, &content, item, plate);
        let viewer = session::me(world);
        details_named(ui, world, &content, item, |world, requirement| {
            let viewer = viewer?;
            Some(condition::holds(world, viewer, requirement))
        });
        grants(ui, world, &content, item, plate);
    });
    chat::offered(world, &shown, ChatEmbed::Item(item));
    shown
}

/// The colour behind the picture on an item's card, by the type of item: it stands in place of
/// the ground of the card style the item's rarity names.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemTypeArtColors {
    pub equipment: ColorGradient,
    pub usable: ColorGradient,
    pub resource: ColorGradient,
}

impl ItemTypeArtColors {
    pub fn of(&self, kind: &ItemKind) -> ColorGradient {
        match kind {
            ItemKind::Equipment { .. } => self.equipment,
            ItemKind::Usable { .. } => self.usable,
            ItemKind::Resource => self.resource,
        }
    }
}

/// The face of an item's card: drawn in the style its rarity names, on its item type's colour
/// behind the picture.
pub fn face_of<'a>(
    content: &'a Content,
    images: &mut (impl ImageSource + ?Sized),
    item: ItemId,
    look: CardLook,
    band: &'a str,
) -> Card<'a> {
    let def = item.get(content);
    let icon = images.image(&def.icon).map(|image| image.texture);
    let ground = content
        .built::<GameSettings>()
        .item_type_art_colors
        .of(&def.kind);
    let style = CardStyleDef::of(content, def.rarity.get(content).card_style);
    let messages = messages::of(content);
    let mut card = Card::new(kind(def), band, &def.name);
    card.dress = style.dress(images, Some(ground));
    card.picture = icon;
    card.close = look.closable.then_some(messages.interface.close());
    card.depth = look.depth;
    card
}

pub fn summary(ui: &mut Ui, content: &Content, item: ItemId, plate: Plate) {
    let def = item.get(content);
    if plate.name_cut {
        card::full_name(ui, &def.name);
    }
    if def.has(ItemModuleKind::QuestItem) {
        controls::pill(ui, messages::of(content).item.quest_item(), Pill::Gold);
    }
}

/// `meets` answers whether the reader meets a requirement, `None` when nobody in particular is
/// reading.
pub fn details(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    meets: impl Fn(&Expr) -> Option<bool>,
) {
    details_drawn(ui, content, item, |ui, requirement, line| {
        let met = requirement.and_then(&meets);
        let words = line.plain_words(content);
        match met {
            Some(met) => card::requirement(ui, met, &words),
            None => card::fact(ui, &words),
        }
    });
}

/// `details` for a reader in the game, whose lines name what they mention.
pub fn details_named(
    ui: &mut Ui,
    world: &mut World,
    content: &Content,
    item: ItemId,
    meets: impl Fn(&World, &Expr) -> Option<bool>,
) {
    details_drawn(ui, content, item, |ui, requirement, line| {
        let met = requirement.and_then(|requirement| meets(world, requirement));
        detail_line(ui, world, met, line);
    });
}

/// A line of a card's facts: a requirement with the mark of whether it is met, or else a fact.
pub fn detail_line(ui: &mut Ui, world: &mut World, met: Option<bool>, line: &LineText) {
    let Some(met) = met else {
        let look = TextLook::role(TextRole::CardBody, ink::INK);
        inline::show_line(ui, world, line, look, None);
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (mark, color) = if met {
            (Mark::Check, ink::VERDIGRIS)
        } else {
            (Mark::Cross, ink::VERMILION)
        };
        glyph::show(ui, mark, 12.0, color);
        let look = TextLook::role(TextRole::CardBody, color);
        inline::show_line(ui, world, line, look, None);
    });
}

/// Draws an item's flavour and facts, each fact by `line`, with the requirement it states where
/// it states one.
fn details_drawn(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    mut line: impl FnMut(&mut Ui, Option<&Expr>, &LineText),
) {
    let def = item.get(content);
    if let Some(flavor) = def.flavor() {
        card::flavour(ui, flavor);
    }
    facts(ui, content, item, def, &mut line);
}

/// The skills an item lends while worn. Answers each one's line, for a reader who may take the
/// skill from it.
pub fn grant_lines(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    item: ItemId,
    plate: Plate,
) -> Vec<(Response, SkillId)> {
    let granted = skill::grants_of(content, item);
    if granted.is_empty() {
        return Vec::new();
    }
    let messages = messages::of(content);
    card::heading(ui, plate, messages.item.grants());
    let lines = granted
        .iter()
        .map(|&granted| {
            (
                grant_line(ui, images, &messages.skill, granted.get(content)),
                granted,
            )
        })
        .collect();
    if granted.iter().any(|skill| skill.get(content).basic_attack) {
        card::foot(ui, messages.item.lends_basic_attack());
    }
    lines
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardLook {
    pub closable: bool,
    pub depth: Depth,
}

impl CardLook {
    pub const PINNED: CardLook = CardLook {
        closable: true,
        depth: Depth::Front,
    };

    /// A card that is part of what stands around it, and goes when that does.
    pub const DOCKED: CardLook = CardLook {
        closable: false,
        depth: Depth::Front,
    };
}

pub fn foot(ui: &mut Ui, world: &World, hint: &StyledLine) {
    ui.add_space(2.0);
    let look = TextLook::new(TextRole::Small.size(), ink::INK3);
    text::show(ui, world, hint, look, None);
}

fn draw_card(world: &mut World) {
    let material = |world: &World, item: &ItemId| {
        let content = world.resource::<Content>();
        let rarity = item.get(content).rarity.get(content);
        CardStyleDef::of(content, rarity.card_style)
            .material
            .clone()
    };
    hud::draw_pinned::<ItemId>(
        world,
        WindowKey::ItemCard,
        material,
        |ui, world, item, depth| {
            let look = CardLook {
                depth,
                ..CardLook::PINNED
            };
            show(ui, world, *item, look)
        },
    );
}

fn grants(ui: &mut Ui, world: &mut World, content: &Content, item: ItemId, plate: Plate) {
    for (response, granted) in grant_lines(ui, world, content, item, plate) {
        let response = response.interact(egui::Sense::click_and_drag());
        if map::drag_started(world, InputAction::CarryItem, &response) {
            drag::carry(ui.ctx(), Carried::Skill(granted));
        } else if map::clicked(world, InputAction::PinToBar, &response) {
            skill::bar::pin(world, skill::HotbarEntry::Skill(granted));
        }
        let hint = slots(world, granted).then(|| {
            messages::of(content)
                .item
                .grant_hint(InputAction::CarryItem, InputAction::PinToBar)
                .styled(content)
        });
        skill::card::tooltip(
            &response,
            world,
            &skill::card::of(world, granted),
            hint.as_ref(),
        );
    }
}

/// Whether the player can put `granted` on the bar: they know it, as while they wear what grants
/// it, and it isn't a passive.
fn slots(world: &World, granted: skill::SkillId) -> bool {
    let content = world.resource::<Content>();
    session::me(world).is_some_and(|me| skill::known(world, me).contains_key(&granted))
        && granted.get(content).kind != skill::SkillKind::Passive
}

fn grant_line(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    words: &skill::SkillMessages,
    def: &SkillDef,
) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        hud::icon(ui, images, &def.icon, card::GRANT_ICON);
        ui.label(face::bold(&def.name).size(TextRole::CardBody.size()));
        ui.label(
            TextRole::CardBody
                .rich(def.target.named(words))
                .color(ink::INK2),
        );
        if def.basic_attack {
            controls::pill(ui, words.basic_attack_mark(), Pill::Gold);
        }
    })
    .response
}

fn facts(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    def: &ItemDef,
    line: &mut impl FnMut(&mut Ui, Option<&Expr>, &LineText),
) {
    let boosts = status::boosts_of(content, item);
    let messages = messages::of(content);
    match &def.kind {
        ItemKind::Equipment { wear, requires, .. } => {
            card::fact(ui, &worn_as(wear, &messages.item));
            for boost in &boosts {
                card::fact(ui, &boost_fact(*boost, false, messages));
            }
            if let Some(requirement) = requires {
                let requires =
                    messages
                        .item
                        .requires(requirement::describe(content, messages, requirement));
                line(ui, Some(requirement), &requires);
            }
        }
        ItemKind::Usable { casts: None } => {}
        ItemKind::Usable { casts: Some(skill) } => {
            let casts = messages.item.casts(Mention::Skill(*skill).span(content));
            line(ui, None, &casts);
        }
        ItemKind::Resource => {
            for boost in &boosts {
                card::fact(ui, &boost_fact(*boost, true, messages));
            }
        }
    }
    match def.stack_max() {
        1 => {}
        u32::MAX => card::fact(ui, messages.item.stacks_without_limit()),
        max => card::fact(ui, &messages.item.stacks_up_to(max)),
    }
    if def.bound() {
        card::fact(ui, messages.item.bound_slot());
    }
}

fn boost_fact(
    (stat, by): (StatKind, Option<f32>),
    carried: bool,
    messages: &messages::Messages,
) -> String {
    let stat = stat.named(&messages.stat);
    let item = &messages.item;
    match (by, carried) {
        (Some(by), false) => item.boost_by(by.abs(), Direction::of(by), stat),
        (None, false) => item.boosts(stat),
        (Some(by), true) => item.carried_boost_by(by.abs(), Direction::of(by), stat),
        (None, true) => item.carried_boosts(stat),
    }
}
