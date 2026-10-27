use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::card::{self, Art, Card, CardResponse, Plate};
use ui::controls::{self, Pill};
use ui::drag;
use ui::egui;
use ui::egui::{Rect, Response, TextureId, Ui};
use ui::face::{self, TextRole};
use ui::glyph::Kind;
use ui::ink;
use ui::sheet::Depth;

use crate::core::content::Content;
use crate::core::gui::{self, Band, ImageSource};
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::hud::{self, Carried};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::item::{Inventory, ItemDef, ItemId, ItemKind, ItemModuleKind, Wear};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::script::{Condition, condition};
use crate::systems::skill::{self, SkillDef, SkillId};
use crate::systems::stat::StatKind;
use crate::systems::status;

const WINDOW_KEY: &str = "Item";

pub struct ItemCardPlugin;

impl Plugin for ItemCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<hud::Pinned<ItemId>>()
            .add_systems(
                Update,
                hud::raise_pinned::<ItemId>(WINDOW_KEY).run_if(in_state(ClientScreen::Area)),
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
    tooltip(world, &response, item);
    response
}

pub fn tooltip(world: &World, response: &Response, item: ItemId) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let lines = [overview(def)];
    let hint = StyledLine::new(inspect_hint());
    hud::tooltip(response, world, &def.name, &lines, Some(&hint));
}

pub fn overview(def: &ItemDef) -> String {
    let category = def.category().label();
    match &def.kind {
        ItemKind::Equipment { wear, .. } => format!("{category} · {}", wear.label()),
        _ => category.to_owned(),
    }
}

fn worn_as(wear: &Wear) -> String {
    match wear {
        Wear::Held { .. } => "Held in the hand".to_owned(),
        Wear::Worn { slot, .. } => format!("Worn on your {}", slot.as_str().replace('_', " ")),
    }
}

pub fn inspect_hint() -> Vec<TextPiece> {
    vec![
        input(InputAction::InspectItem),
        TextPiece::text(" for more information"),
    ]
}

pub fn use_verb(def: &ItemDef) -> Option<&'static str> {
    match def.kind {
        ItemKind::Usable => Some("use"),
        ItemKind::Equipment { .. } => Some("wear"),
        ItemKind::Resource => None,
    }
}

pub fn kind(def: &ItemDef) -> Kind {
    match def.kind {
        ItemKind::Equipment { .. } => Kind::Equipment,
        ItemKind::Usable => Kind::Usable,
        ItemKind::Resource => Kind::Resource,
    }
}

pub fn show(ui: &mut Ui, world: &mut World, item: ItemId, look: CardLook) -> CardResponse {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let held = session::me(world)
        .and_then(|me| world.get::<Inventory>(me))
        .map_or(0, |bag| bag.count(item));
    let numeral = (def.stack_max() > 1 && held > 0).then(|| face::roman(held));
    let icon = gui::image(world, &def.icon).map(|image| image.texture);
    let mut card = face_of(def, icon, look);
    card.numeral = numeral.as_deref();
    card.show(ui, |ui, plate| {
        summary(ui, &content, item, plate);
        let viewer = session::me(world);
        details(ui, &content, item, |requirement| {
            let viewer = viewer?;
            Some(if condition::holds(world, viewer, requirement) {
                Judgement::Met
            } else {
                Judgement::Unmet {
                    standing: condition::standing(world, viewer, requirement),
                }
            })
        });
        grants(ui, world, &content, item);
        if let Some(verb) = use_verb(def) {
            let hint = StyledLine::new(vec![
                input(InputAction::UseItem),
                TextPiece::text(format!(" in your bag to {verb}")),
            ]);
            foot(ui, world, &hint);
        }
    })
}

pub fn face_of(def: &ItemDef, icon: Option<TextureId>, look: CardLook) -> Card<'_> {
    let mut card = Card::new(kind(def), &def.name);
    card.art = Art {
        icon,
        height: look.art,
    };
    card.width = look.width;
    card.max_height = look.max_height;
    card.closable = look.closable;
    card.depth = look.depth;
    card
}

pub enum Judgement {
    Met,
    Unmet { standing: Option<String> },
}

pub fn summary(ui: &mut Ui, content: &Content, item: ItemId, plate: Plate) {
    let def = item.get(content);
    if plate.name_cut {
        card::full_name(ui, &def.name);
    }
    card::overview(ui, &overview(def));
    let tags = tags(def);
    if !tags.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (label, pill) in tags {
                controls::pill(ui, label, pill);
            }
        });
    }
}

/// `judge` answers `None` when nobody in particular is reading.
pub fn details(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    judge: impl Fn(&Condition) -> Option<Judgement>,
) {
    let def = item.get(content);
    if let Some(flavor) = def.flavor() {
        card::flavour(ui, flavor);
    }
    facts(ui, content, item, def, judge);
}

/// The skills an item lends while worn. Answers each one's line, for a reader who may take the
/// skill from it.
pub fn grant_lines(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    item: ItemId,
) -> Vec<(Response, SkillId)> {
    let granted = skill::grants_of(content, item);
    if granted.is_empty() {
        return Vec::new();
    }
    card::heading(ui, "grants while worn");
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
    pub width: f32,
    pub max_height: f32,
    pub art: f32,
    pub closable: bool,
    pub depth: Depth,
}

impl CardLook {
    pub const PINNED: CardLook = CardLook {
        width: card::WIDTH,
        max_height: card::MAX_HEIGHT,
        art: card::ART,
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
    hud::draw_pinned::<ItemId>(world, WINDOW_KEY, |ui, world, item, depth| {
        let look = CardLook {
            depth,
            ..CardLook::PINNED
        };
        show(ui, world, *item, look)
    });
}

fn grants(ui: &mut Ui, world: &mut World, content: &Content, item: ItemId) {
    for (response, granted) in grant_lines(ui, world, content, item) {
        let response = response.interact(egui::Sense::click_and_drag());
        if map::drag_started(world, InputAction::CarryItem, &response) {
            drag::carry(ui.ctx(), Carried::Skill(granted));
        } else if map::clicked(world, InputAction::PinToHotbar, &response) {
            skill::bar::pin(world, skill::HotbarEntry::Skill(granted));
        }
        let hint = StyledLine::new(vec![
            input(InputAction::CarryItem),
            TextPiece::text(" carries it to a slot · "),
            input(InputAction::PinToHotbar),
            TextPiece::text(" puts it on the hotbar"),
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

fn tags(def: &ItemDef) -> Vec<(&'static str, Pill)> {
    [
        (ItemModuleKind::Bound, "Bound", Pill::Rubric),
        (ItemModuleKind::QuestItem, "Quest item", Pill::Gold),
    ]
    .into_iter()
    .filter(|(flag, ..)| def.has(*flag))
    .map(|(_, label, pill)| (label, pill))
    .collect()
}

fn facts(
    ui: &mut Ui,
    content: &Content,
    item: ItemId,
    def: &ItemDef,
    judge: impl Fn(&Condition) -> Option<Judgement>,
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
                match judge(requirement) {
                    Some(Judgement::Met) => card::requirement(ui, true, &line),
                    Some(Judgement::Unmet {
                        standing: Some(standing),
                    }) => {
                        card::requirement(ui, false, &format!("{line} · {standing}"));
                    }
                    Some(Judgement::Unmet { standing: None }) => {
                        card::requirement(ui, false, &line)
                    }
                    None => card::fact(ui, &line),
                }
            }
        }
        ItemKind::Usable => card::fact(ui, "Can be used"),
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
