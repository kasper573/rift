use bevy::prelude::*;
use ui::drag;
use ui::egui;
use ui::egui::{Align, Color32, Layout, Rect, Sense, Ui, Vec2};
use ui::face::TextRole;
use ui::float;
use ui::{controls, glyph, ink};

use crate::core::content::Content;
use crate::core::gui::ImageSource;
use crate::core::text::LineText;
use crate::core::text::StyledLine;
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::stance::{self, HandIndex, HandRole, StanceDef};
use crate::systems::actor::{Appearance, appearance};
use crate::systems::equipment::{EquipRequest, EquipmentPlace, UnequipRequest};
use crate::systems::hud::Carried;
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::widget::{ItemSlotLook, SLOT, slot, slot_pip};
use crate::systems::item::{ItemId, card};
use crate::systems::messages;
use crate::systems::player::session;

const SLOT_GAP: f32 = 5.0;
const NAMED_SLOT: f32 = 46.0;
pub const PANE_WIDTH: f32 = 5.0 * NAMED_SLOT + 4.0 * SLOT_GAP;

pub fn pane(ui: &mut Ui, world: &mut World) {
    let places = places(world);
    let shown = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(SLOT_GAP, 2.0);
            // A place stands taller than its slot by its name, so the row is laid from its top:
            // centred, each place would stand lower than the one before it.
            let row = Layout::left_to_right(Align::Min).with_main_wrap(true);
            let room = Vec2::new(ui.available_width(), SLOT);
            ui.allocate_ui_with_layout(room, row, |ui| {
                for place in &places {
                    let span = place.span() as f32;
                    let width = NAMED_SLOT * span + SLOT_GAP * (span - 1.0);
                    ui.allocate_ui(Vec2::new(width, SLOT), |ui| {
                        ui.vertical_centered(|ui| {
                            match place.item {
                                Some(item) => worn_slot(ui, world, place, item),
                                None => empty_slot(ui, world, place),
                            }
                            ui.add(
                                egui::Label::new(
                                    TextRole::Small.rich(place.label()).color(ink::INK3),
                                )
                                .truncate()
                                .show_tooltip_when_elided(false),
                            );
                        });
                    });
                }
            });
        })
        .response;
    if let Some(Carried::BagSlot(slot)) = drag::dropped::<Carried>(&shown) {
        world.write_message(EquipRequest { slot, hand: None });
    }
}

/// A place on the pane: a hand of the body, two hands a two-hander spans, a slot of its
/// skeleton, or the tool slot.
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

    fn kind(&self) -> &'static str {
        match (&self.slot, self.names.len()) {
            (EquipmentPlace::Slot(_), _) => "slot",
            (EquipmentPlace::Tool, _) => "tool",
            (_, 2) => "both_hands",
            _ if self.unseen => "any_hand",
            _ => "hand",
        }
    }
}

/// Every hand of the character's body in its skeleton's order, a two-hander spanning the hands it
/// takes, every slot of its skeleton, and the tool slot.
fn places(world: &World) -> Vec<Place> {
    let Some(me) = session::me(world) else {
        return Vec::new();
    };
    let messages = &messages::of(world.resource::<Content>()).equipment;
    let hands = held::hands_of(world, me);
    let names: Vec<String> = match hands.len() {
        0 => vec![messages.hand_name().to_owned()],
        _ => hands.iter().map(|hand| hand.label().to_owned()).collect(),
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
    let skeleton = appearance::model_of(world, me).map(|model| &model.skeleton().def);
    for slot in skeleton.map_or(&[][..], |skeleton| &skeleton.slots) {
        let place = EquipmentPlace::Slot(slot.clone());
        let item = super::in_slot(world, me, &place);
        let unseen = item
            .zip(model)
            .is_some_and(|(item, model)| appearance::worn_parts(content, item, model).is_empty());
        places.push(Place {
            slot: place,
            names: vec![
                skeleton
                    .map_or(slot.as_str(), |skeleton| skeleton.slot_label(slot))
                    .to_owned(),
            ],
            item,
            unseen,
        });
    }
    places.push(Place {
        item: super::in_slot(world, me, &EquipmentPlace::Tool),
        slot: EquipmentPlace::Tool,
        names: vec![messages.tool_name().to_owned()],
        unseen: false,
    });
    places
}

fn empty_slot(ui: &mut Ui, world: &mut World, place: &Place) {
    let content = world.resource::<Content>().clone();
    let messages = &messages::of(&content).equipment;
    let response = framed(ui, world, place, ItemSlotLook::EMPTY, Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            messages.empty_named(place.label()),
        )
    });
    held_where_dropped(world, place, &response);
    let line = match &place.slot {
        EquipmentPlace::Slot(_) => messages.empty_slot_tip().to_owned(),
        EquipmentPlace::Tool => messages.empty_tool_tip().to_owned(),
        EquipmentPlace::Hand(hand) => match landing(&content, *hand) {
            Some(lands) => messages.empty_hand_landing_tip(lands),
            None => messages.empty_hand_tip().to_owned(),
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
    let messages = &messages::of(&content);
    let faded = drag::carried::<Carried>(ui.ctx()) == Some(Carried::Worn(place.slot.clone()));
    let look = ItemSlotLook {
        faded,
        ..ItemSlotLook::of(&content, item)
    };
    let response = framed(ui, world, place, look, Sense::click_and_drag());
    held_where_dropped(world, place, &response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            messages.equipment.worn_as(&def.name, place.label()),
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
    let equipment = &messages.equipment;
    let hint = StyledLine::new(map::hints(card::inspect_hints(world).into_iter().chain([
        (InputAction::UseItem, equipment.take_off().to_owned()),
        (InputAction::CarryItem, equipment.carry_off().to_owned()),
    ])));
    let kind = place.kind();
    let mut lines = Vec::new();
    match stance::of_item(&content, item) {
        Some(stance) if place.unseen && place.slot != EquipmentPlace::Tool => {
            lines.push(equipment.unseen_stance(&stance.get(&content).name))
        }
        Some(stance) => lines.push(equipment.with_stance(kind, &stance.get(&content).name)),
        None if place.unseen => lines.push(equipment.unseen(kind)),
        None => lines.push(equipment.where_held(kind)),
    }
    if matches!(place.slot, EquipmentPlace::Hand(_)) && fights_unarmed(world) {
        lines.push(equipment.no_set().to_owned());
    }
    card::chat_shortcut(world, &response, &[item]);
    let lines = lines.into_iter().map(LineText::from);
    card::tooltip_noted(world, &response, item, lines, Some(&hint));
}

/// Holds a bag's item dropped on a hand in that hand.
fn held_where_dropped(world: &mut World, place: &Place, response: &egui::Response) {
    if let EquipmentPlace::Hand(hand) = place.slot
        && let Some(Carried::BagSlot(slot)) = drag::dropped::<Carried>(response)
    {
        world.write_message(EquipRequest {
            slot,
            hand: Some(hand),
        });
    }
}

/// A slot as wide as the hands it spans, hatched when the body cannot show what it holds.
fn framed(
    ui: &mut Ui,
    world: &mut World,
    place: &Place,
    look: ItemSlotLook,
    sense: Sense,
) -> egui::Response {
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
            filled: look.icon.is_some(),
            hovered: response.hovered(),
            refused: false,
            edge: look.edge(),
        },
    );
    slot_pip(ui, world, rect, &look);
    if let Some(image) = look.icon.and_then(|icon| world.image(icon)) {
        let tint = if look.faded {
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
        .map(|stance| stance.name.clone())
        .collect();
    (!names.is_empty()).then(|| messages::of(content).interface.listed_words(names))
}

/// Whether the character's stance has no set on its body, so it fights with the base set.
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
