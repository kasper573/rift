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
use ui::glyph::CardKind;
use ui::ink;
use ui::sheet::Depth;

use crate::core::card::CardStyleDef;
use crate::core::content::Content;
use crate::core::gui::{self, Band, ImageSource};
use crate::core::math::ColorGradient;
use crate::core::stage::card::HungCardLamp;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::GameSettings;
use crate::systems::chat::ChatEmbed;
use crate::systems::hud::{self, Carried};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::item::{Inventory, ItemDef, ItemId, ItemKind, ItemModuleKind, ItemShown, Wear};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::{Condition, condition};
use crate::systems::skill::{self, SkillDef, SkillId};
use crate::systems::stat::StatKind;
use crate::systems::status;
use crate::systems::terminal::chat;

pub(crate) const WINDOW_KEY: &str = "Item";

pub struct ItemCardPlugin;

impl Plugin for ItemCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<hud::Pinned<ItemId>>()
            .add_systems(
                Update,
                (show_told, hud::raise_pinned::<ItemId>(WINDOW_KEY))
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
    hud::window_layer(WINDOW_KEY)
}

/// How far the pinned card is turned toward the pointer.
pub fn tilt(world: &World) -> CardTilt {
    hud::pinned_tilt(world, WINDOW_KEY)
}

/// The lamp the pinned card is lit by, while it is on the stage.
pub fn lamp(world: &World) -> Option<HungCardLamp> {
    hud::pinned_lamp(world, WINDOW_KEY)
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
    let hint = StyledLine::new(inspect_hint());
    tooltip_noted(world, response, item, [], Some(&hint));
}

/// An item's tooltip: its name, a line led by its rarity that says what it is, `notes` under
/// that, and a hint.
pub fn tooltip_noted(
    world: &mut World,
    response: &Response,
    item: ItemId,
    notes: impl IntoIterator<Item = String>,
    hint: Option<&StyledLine>,
) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let rarity = def.rarity.get(&content);
    let mark = rarity
        .icon
        .as_ref()
        .and_then(|icon| gui::image(world, icon));
    let lines: Vec<String> = std::iter::once(format!("{} · {}", rarity.name, overview(def)))
        .chain(notes)
        .collect();
    hud::tooltip_marked(response, world, &def.name, mark.as_ref(), &lines, hint);
}

/// What an item is, in a line, for a tooltip, which has no band or style to say it: led by the
/// rarity's name there. A card says each part of it once elsewhere.
fn overview(def: &ItemDef) -> String {
    let category = def.category().label();
    match &def.kind {
        ItemKind::Equipment { wear, .. } => format!("{category} · {}", wear.label()),
        _ => category.to_owned(),
    }
}

fn worn_as(wear: &Wear) -> String {
    match wear {
        Wear::Held { .. } => "Held in the hand".to_owned(),
        Wear::Tool { .. } => "Kept in the tool slot".to_owned(),
        Wear::Worn { slot, .. } => format!("Worn on your {}", slot.as_str().replace('_', " ")),
    }
}

pub fn inspect_hint() -> Vec<TextPiece> {
    let mut hint = vec![
        input(InputAction::InspectItem),
        TextPiece::text(" for more information · "),
    ];
    hint.extend(chat::shortcut_hint());
    hint
}

pub fn use_verb(def: &ItemDef) -> Option<&'static str> {
    match def.kind {
        ItemKind::Usable { .. } => Some("use"),
        ItemKind::Equipment { .. } => Some("wear"),
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
    let held = session::me(world)
        .and_then(|me| world.get::<Inventory>(me))
        .map_or(0, |bag| bag.count(item));
    let tally = (def.stack_max() > 1 && held > 0).then(|| format!("{held} held"));
    let mut card = face_of(&content, world, item, look);
    card.tally = tally.as_deref();
    card.action = Some(chat::card_action());
    let shown = card.show(ui, |ui, plate| {
        summary(ui, &content, item, plate);
        let viewer = session::me(world);
        details(ui, &content, item, |requirement| {
            let viewer = viewer?;
            Some(condition::holds(world, viewer, requirement))
        });
        grants(ui, world, &content, item, plate);
        if let Some(verb) = use_verb(def) {
            let hint = StyledLine::new(vec![
                input(InputAction::UseItem),
                TextPiece::text(format!(" in your inventory to {verb}")),
            ]);
            foot(ui, world, &hint);
        }
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
) -> Card<'a> {
    let def = item.get(content);
    let icon = images.image(&def.icon).map(|image| image.texture);
    let ground = content
        .built::<GameSettings>()
        .item_type_art_colors
        .of(&def.kind);
    let style = CardStyleDef::of(content, def.rarity.get(content).card_style);
    let mut card = Card::new(kind(def), &def.name);
    card.band = def.category().label();
    card.dress = style.dress(images, Some(ground));
    card.picture = icon;
    card.closable = look.closable;
    card.depth = look.depth;
    card
}

pub fn summary(ui: &mut Ui, content: &Content, item: ItemId, plate: Plate) {
    let def = item.get(content);
    if plate.name_cut {
        card::full_name(ui, &def.name);
    }
    if def.has(ItemModuleKind::QuestItem) {
        controls::pill(ui, "Quest item", Pill::Gold);
    }
}

/// `meets` answers whether the reader meets a requirement, `None` when nobody in particular is
/// reading.
pub fn details(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    meets: impl Fn(&Condition) -> Option<bool>,
) {
    let def = item.get(content);
    if let Some(flavor) = def.flavor() {
        card::flavour(ui, flavor);
    }
    facts(ui, content, item, def, meets);
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
    card::heading(ui, plate, "grants while worn");
    let lines = granted
        .iter()
        .map(|&granted| (grant_line(ui, images, granted.get(content)), granted))
        .collect();
    if granted.iter().any(|skill| skill.get(content).basic_attack) {
        card::foot(
            ui,
            "On the basic attack slot, it lends its first basic attack",
        );
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
    hud::draw_pinned::<ItemId>(world, WINDOW_KEY, material, |ui, world, item, depth| {
        let look = CardLook {
            depth,
            ..CardLook::PINNED
        };
        show(ui, world, *item, look)
    });
}

fn grants(ui: &mut Ui, world: &mut World, content: &Content, item: ItemId, plate: Plate) {
    for (response, granted) in grant_lines(ui, world, content, item, plate) {
        let response = response.interact(egui::Sense::click_and_drag());
        if map::drag_started(world, InputAction::CarryItem, &response) {
            drag::carry(ui.ctx(), Carried::Skill(granted));
        } else if map::clicked(world, InputAction::PinToBar, &response) {
            skill::bar::pin(world, skill::HotbarEntry::Skill(granted));
        }
        let hint = StyledLine::new(vec![
            input(InputAction::CarryItem),
            TextPiece::text(" carries it to a slot · "),
            input(InputAction::PinToBar),
            TextPiece::text(" puts it on the bar"),
        ]);
        skill::card::tooltip(
            &response,
            world,
            &skill::card::of(world, granted),
            Some(&hint),
        );
    }
}

fn grant_line(ui: &mut Ui, images: &mut (impl ImageSource + ?Sized), def: &SkillDef) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        hud::icon(ui, images, &def.icon, card::GRANT_ICON);
        ui.label(face::bold(&def.name).size(TextRole::CardBody.size()));
        ui.label(
            TextRole::CardBody
                .rich(def.target.describe())
                .color(ink::INK2),
        );
        if def.basic_attack {
            controls::pill(ui, skill::card::BASIC_ATTACK_MARK, Pill::Gold);
        }
    })
    .response
}

fn facts(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    def: &ItemDef,
    meets: impl Fn(&Condition) -> Option<bool>,
) {
    let boosts = status::boosts_of(content, item);
    match &def.kind {
        ItemKind::Equipment { wear, requires, .. } => {
            card::fact(ui, &worn_as(wear));
            for boost in &boosts {
                card::fact(ui, &modifier(*boost));
            }
            if let Some(requirement) = requires {
                let line = format!("Requires {}", condition::describe(content, requirement));
                match meets(requirement) {
                    Some(met) => card::requirement(ui, met, &line),
                    None => card::fact(ui, &line),
                }
            }
        }
        ItemKind::Usable { casts: None } => {}
        ItemKind::Usable { casts: Some(skill) } => {
            card::fact(ui, &format!("Casts {}", skill.get(content).name))
        }
        ItemKind::Resource => {
            for boost in &boosts {
                card::fact(ui, &format!("{} while carried", modifier(*boost)));
            }
        }
    }
    match def.stack_max() {
        1 => {}
        u32::MAX => card::fact(ui, "Stacks without limit"),
        max => card::fact(ui, &format!("Stacks up to {max}")),
    }
    if def.bound() {
        card::fact(ui, "Can't be sold or dropped");
    }
}

fn modifier((stat, by): (StatKind, Option<f32>)) -> String {
    let name: &'static str = stat.into();
    match by {
        Some(by) => format!("{by:+} {name}"),
        None => format!("Boosts {name}"),
    }
}
