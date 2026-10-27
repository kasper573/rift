use bevy::prelude::*;
use ui::drag;
use ui::egui;
use ui::egui::{Color32, Rect, Sense, Ui, Vec2};
use ui::face::TextRole;
use ui::float;
use ui::{controls, glyph, ink, sheet};

use crate::core::assets::ImageRef;
use crate::core::content::Content;
use crate::core::gui::ImageSource;
use crate::core::text::{StyledLine, TextPiece};
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::skeleton::EquipmentSlot;
use crate::systems::actor::stance::{self, HandIndex, HandRole, StanceDef};
use crate::systems::actor::{Appearance, appearance};
use crate::systems::equipment::{EquipmentPlace, UnequipRequest};
use crate::systems::hud::{self, Carried, HudWindow};
use crate::systems::input::map::{self, InputAction, input};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::widget::{SLOT, SlotLook, slot};
use crate::systems::item::{ItemId, UseItemRequest, card};
use crate::systems::player::session;

const SLOT_GAP: f32 = 8.0;
const NAMED_SLOT: f32 = 52.0;

pub struct EquipmentWindow;

impl HudWindow for EquipmentWindow {
    fn title(&self) -> &'static str {
        "Equipment"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleEquipment
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Equipment
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let places = places(world);
        let shown = ui
            .scope(|ui| {
                sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(SLOT_GAP, 2.0);
                    ui.horizontal_wrapped(|ui| {
                        for place in &places {
                            let span = place.span() as f32;
                            let width = NAMED_SLOT * span + SLOT_GAP * (span - 1.0);
                            ui.allocate_ui(Vec2::new(width, NAMED_SLOT), |ui| {
                                ui.vertical_centered(|ui| {
                                    match place.item {
                                        Some(item) => worn_slot(ui, world, place, item),
                                        None => empty_slot(ui, world, place),
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            TextRole::Small.rich(place.label()).color(ink::INK3),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        }
                    });
                });
            })
            .response;
        if let Some(Carried::BagSlot(slot)) = drag::dropped::<Carried>(&shown) {
            world.write_message(UseItemRequest { slot });
        }
    }
}

/// A place on the window: a hand of the body, two hands a two-hander spans, or a slot of its
/// skeleton.
struct Place {
    slot: EquipmentPlace,
    names: Vec<String>,
    item: Option<ItemId>,
    /// Whether the body cannot show what is worn here.
    unseen: bool,
}

impl Place {
    fn span(&self) -> usize {
        self.names.len().max(1)
    }

    fn label(&self) -> String {
        self.names.join(" · ")
    }

    /// Where the item is worn, in a sentence.
    fn where_held(&self) -> String {
        match (&self.slot, self.names.len()) {
            (EquipmentPlace::Slot(slot), _) => format!("Worn on the {}", spoken(slot)),
            (_, 2) => "Held in both hands".to_owned(),
            _ => format!(
                "Held in the {}",
                self.names
                    .first()
                    .map_or("hand", String::as_str)
                    .to_lowercase()
            ),
        }
    }
}

/// Every hand of the player's body in its skeleton's order, a two-hander spanning the hands it
/// takes, and every slot of its skeleton.
fn places(world: &World) -> Vec<Place> {
    let Some(me) = session::me(world) else {
        return Vec::new();
    };
    let hands = held::hands_of(world, me);
    let names: Vec<String> = match hands.len() {
        0 => vec!["Hand".to_owned()],
        _ => hands.iter().map(|hand| capitalized(&hand.name)).collect(),
    };
    let unseen = hands.is_empty();
    let mut places: Vec<Place> = Vec::new();
    for (index, name) in names.into_iter().enumerate() {
        let slot = EquipmentPlace::Hand(HandIndex(index as u8));
        let item = super::in_slot(world, me, &slot);
        if let Some(last) = places.last_mut()
            && item.is_some()
            && last.item == item
        {
            last.names.push(name);
            continue;
        }
        places.push(Place {
            slot,
            names: vec![name],
            item,
            unseen,
        });
    }
    let content = world.resource::<Content>();
    let model = world
        .get::<Appearance>(me)
        .map(|appearance| appearance.model);
    for slot in super::body_slots(world, me) {
        let place = EquipmentPlace::Slot(slot.clone());
        let item = super::in_slot(world, me, &place);
        let unseen = item
            .zip(model)
            .is_some_and(|(item, model)| appearance::worn_parts(content, item, model).is_empty());
        places.push(Place {
            slot: place,
            names: vec![capitalized(&spoken(&slot))],
            item,
            unseen,
        });
    }
    places
}

fn empty_slot(ui: &mut Ui, world: &mut World, place: &Place) {
    let response = framed(ui, world, place, None, false, Sense::hover());
    let content = world.resource::<Content>().clone();
    let line = match &place.slot {
        EquipmentPlace::Slot(slot) => {
            format!("Empty · what is worn on the {} goes here", spoken(slot))
        }
        EquipmentPlace::Hand(hand) => match landing(&content, *hand) {
            Some(lands) => format!("Empty · {lands} land here"),
            None => "Empty".to_owned(),
        },
    };
    float::tooltip(&response, |ui| {
        ui.label(place.label());
        ui.label(TextRole::Small.rich(line).color(ink::INK3));
    });
}

fn worn_slot(ui: &mut Ui, world: &mut World, place: &Place, item: ItemId) {
    let content = world.resource::<Content>().clone();
    let def = item.get(&content);
    let faded = drag::carried::<Carried>(ui.ctx()) == Some(Carried::Worn(place.slot.clone()));
    let response = framed(
        ui,
        world,
        place,
        Some(&def.icon),
        faded,
        Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Worn as {}: {}", place.label(), def.name),
        )
    });
    if map::clicked(world, InputAction::UseItem, &response) {
        world.write_message(UnequipRequest {
            slot: place.slot.clone(),
        });
    } else if map::clicked(world, InputAction::InspectItem, &response) {
        card::open(world, item, Some(response.rect));
    } else if map::drag_started(world, InputAction::CarryItem, &response) {
        drag::carry(ui.ctx(), Carried::Worn(place.slot.clone()));
    }
    let hint = StyledLine::new(
        card::inspect_hint()
            .into_iter()
            .chain([
                TextPiece::text(" · "),
                input(InputAction::UseItem),
                TextPiece::text(" to take it off · "),
                input(InputAction::CarryItem),
                TextPiece::text(" onto the hotbar"),
            ])
            .collect(),
    );
    let mut lines = vec![card::overview(def)];
    match stance::of_item(&content, item) {
        Some(stance) if place.unseen => lines.push(format!(
            "Worn in the hand · this body cannot show it · {} stance",
            stance.get(&content).name
        )),
        Some(stance) => lines.push(format!(
            "{} · {} stance",
            place.where_held(),
            stance.get(&content).name
        )),
        None if place.unseen => {
            lines.push(format!("{} · this body cannot show it", place.where_held()))
        }
        None => lines.push(place.where_held()),
    }
    if matches!(place.slot, EquipmentPlace::Hand(_)) && fights_unarmed(world) {
        lines.push("Its stance has no set here: you fight as unarmed".to_owned());
    }
    hud::tooltip(&response, world, &def.name, &lines, Some(&hint));
}

/// A slot as wide as the hands it spans, hatched when the body cannot show what it holds.
fn framed(
    ui: &mut Ui,
    world: &mut World,
    place: &Place,
    icon: Option<&ImageRef>,
    faded: bool,
    sense: Sense,
) -> egui::Response {
    let look = SlotLook {
        icon,
        faded,
        ..SlotLook::EMPTY
    };
    if place.span() == 1 {
        let response = slot(ui, world, look, sense);
        if place.unseen {
            glyph::hatch(ui.painter(), response.rect, ink::INK3);
        }
        return response;
    }
    let width = SLOT * place.span() as f32 + SLOT_GAP * (place.span() as f32 - 1.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, SLOT), sense);
    controls::slot_frame(
        ui.painter(),
        rect,
        controls::SlotLook {
            filled: icon.is_some(),
            hovered: response.hovered(),
            refused: false,
        },
    );
    if let Some(image) = icon.and_then(|icon| world.image(icon)) {
        let tint = if faded {
            Color32::from_white_alpha(110)
        } else {
            Color32::WHITE
        };
        ui.place(
            Rect::from_center_size(rect.center(), Vec2::splat(controls::SLOT_ICON)),
            image.widget(Vec2::splat(controls::SLOT_ICON)).tint(tint),
        );
    }
    response
}

/// What lands in `hand` when equipped: the stances made for it, by name.
fn landing(content: &Content, hand: HandIndex) -> Option<String> {
    let role = match hand {
        HandIndex::MAIN => HandRole::Main,
        HandIndex::OFF => HandRole::Off,
        _ => return None,
    };
    let names: Vec<String> = content
        .table::<StanceDef>()
        .rows()
        .filter(|stance| stance.hand == role)
        .map(|stance| stance.name.to_lowercase())
        .collect();
    (!names.is_empty()).then(|| ui::words::join_and(&names))
}

/// Whether the player's stance has no set on its body, so it fights with the base set.
fn fights_unarmed(world: &World) -> bool {
    let Some(me) = session::me(world) else {
        return false;
    };
    let Some(stance) = held::stance_of(world, me) else {
        return false;
    };
    if world.get::<Held>(me).is_none() {
        return false;
    }
    appearance::model_of(world, me).is_some_and(|model| model.skeleton().def.set(stance).is_none())
}

/// A slot's name as words: `left_shoulder` as "left shoulder".
fn spoken(slot: &EquipmentSlot) -> String {
    slot.as_str().replace('_', " ")
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
