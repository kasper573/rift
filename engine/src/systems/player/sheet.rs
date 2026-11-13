use bevy::prelude::*;
use ui::card::CardPane;
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, UiBuilder};
use ui::face::TextRole;
use ui::sheet::Depth;
use ui::{card, controls, face, float, ink, sheet, words};

use crate::core::content::Content;
use crate::systems::actor::Name;
use crate::systems::equipment::{self, EquipmentPlace, UnequipRequest};
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::card::{CardLook, CardNoteTone, ItemCardNote};
use crate::systems::item::widget::{InventoryPane, inventory_pane, pane_width, slot_notes};
use crate::systems::item::{self, DropItemRequest, Inventory, ItemId, ItemKind, ItemStack};
use crate::systems::player::session;
use crate::systems::skill::bar;
use crate::systems::stat::{self, StatKind};
use crate::systems::status;

pub struct CharacterMenuTab;

impl MenuTab for CharacterMenuTab {
    fn name(&self) -> MenuTabName {
        MenuTabName::Character
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Character
    }
    fn opens(&self) -> InputAction {
        InputAction::OpenCharacter
    }
    fn hints(&self) -> &'static [(InputAction, &'static str)] {
        &[
            (InputAction::Select, "select"),
            (InputAction::UseItem, "use or wear"),
            (InputAction::CarryItem, "carry, or put on the bar"),
            (InputAction::DropItem, "drop"),
            (InputAction::InspectItem, "pin the card"),
        ]
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let room = ui.available_rect_before_wrap();
        let worn_room = Rect::from_min_max(
            room.min,
            Pos2::new(
                room.min.x + equipment::widget::PANE_WIDTH + SCROLL_BAR,
                room.max.y,
            ),
        );
        let carried_room = Rect::from_min_max(
            Pos2::new(worn_room.max.x + COLUMN_GAP, room.min.y),
            Pos2::new(
                worn_room.max.x + COLUMN_GAP + pane_width(INVENTORY_COLUMNS),
                room.max.y,
            ),
        );
        let card_room = Rect::from_min_max(
            Pos2::new(carried_room.max.x + COLUMN_GAP, room.min.y),
            room.max,
        );
        let was = world.get_resource_or_init::<CharacterSelection>().0.clone();
        let slot_was = match was {
            Some(Selected::Slot { slot, .. }) => Some(slot),
            _ => None,
        };
        let worn_was = match &was {
            Some(Selected::Worn(place)) => Some(place.clone()),
            _ => None,
        };
        let (mut slot, mut worn) = (slot_was, worn_was.clone());
        column(ui, "worn", worn_room, |ui| {
            if let Some(name) = session::my_character(world).and_then(|me| me.get::<Name>()) {
                ui.label(face::bold(name.name.as_str()));
                ui.add_space(4.0);
            }
            sheet::section(ui, "Equipment");
            equipment::widget::pane(ui, world, &mut worn);
            ui.add_space(SECTION_GAP);
            sheet::section(ui, "Stats");
            stat::widget::pane(ui, world);
        });
        column(ui, "carried", carried_room, |ui| {
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: Some("Inventory"),
                selected: &mut slot,
            };
            inventory_pane(ui, world, pane);
        });
        let picked = if slot != slot_was {
            slot.and_then(|slot| {
                let item = bag_stack(world, slot)?.item;
                Some(Selected::Slot { slot, item })
            })
        } else if worn != worn_was {
            worn.map(Selected::Worn)
        } else {
            was
        };
        let selection = picked.and_then(|selected| Some((stack_of(world, &selected)?, selected)));
        world.resource_mut::<CharacterSelection>().0 =
            selection.as_ref().map(|(_, selected)| selected.clone());
        ui.scope_builder(UiBuilder::new().max_rect(card_room), |ui| {
            selected_pane(ui, world, selection.as_ref());
        });
    }
}

const COLUMN_GAP: f32 = 16.0;
const SECTION_GAP: f32 = 10.0;
const INVENTORY_COLUMNS: u32 = 6;
const SCROLL_BAR: f32 = 14.0;

#[derive(Resource, Default)]
struct CharacterSelection(Option<Selected>);

#[derive(Clone, PartialEq)]
enum Selected {
    // With the item it held when selected: the bag closes up as things leave it, and what
    // slides into the slot was never chosen.
    Slot { slot: u32, item: ItemId },
    Worn(EquipmentPlace),
}

#[derive(Clone, Copy)]
enum Press {
    Use,
    Drop,
    TakeOff,
}

fn column(ui: &mut Ui, id: &str, rect: Rect, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        let area = egui::ScrollArea::vertical()
            .id_salt(("character", id))
            .auto_shrink(false);
        sheet::scroll(ui, area, add);
    });
}

fn bag_stack(world: &World, slot: u32) -> Option<ItemStack> {
    world
        .get::<Inventory>(session::me(world)?)?
        .slots
        .get(slot as usize)
        .copied()
}

fn stack_of(world: &World, selected: &Selected) -> Option<ItemStack> {
    match selected {
        Selected::Slot { slot, item } => {
            bag_stack(world, *slot).filter(|stack| stack.item == *item)
        }
        Selected::Worn(place) => equipment::in_slot(world, session::me(world)?, place)
            .map(|item| ItemStack::new(item, 1)),
    }
}

fn selected_pane(ui: &mut Ui, world: &mut World, selection: Option<&(ItemStack, Selected)>) {
    sheet::section(ui, "Selected");
    let pane = CardPane {
        size: ui.available_size(),
        foot: controls::CONTROL_HEIGHT,
    };
    let offered = selection.map(|(stack, selected)| Offered::of(world, stack.item, selected));
    let mut pressed = None;
    pane.show(
        ui,
        |ui| match selection {
            Some((stack, selected)) => {
                let notes = notes(world, *stack, selected);
                let look = CardLook {
                    width: ui.available_width(),
                    max_height: ui.available_height().min(card::MAX_HEIGHT),
                    art: card::ART_SHORT,
                    closable: false,
                    depth: Depth::Front,
                };
                item::card::show_noted(ui, world, stack.item, look, &notes);
            }
            None => {
                ui.add_space(16.0);
                sheet::label(
                    ui,
                    TextRole::Small,
                    "Select something you wear or carry",
                    ink::INK3,
                );
            }
        },
        |ui| {
            if let Some(offered) = offered {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    pressed = offered.show(ui);
                });
            }
        },
    );
    match (pressed, selection.map(|(_, selected)| selected)) {
        (Some(Press::Use), Some(Selected::Slot { slot, .. })) => {
            bar::use_bag_slot(world, *slot);
        }
        (Some(Press::Drop), Some(Selected::Slot { slot, .. })) => {
            world.write_message(DropItemRequest { slot: *slot });
        }
        (Some(Press::TakeOff), Some(Selected::Worn(place))) => {
            world.write_message(UnequipRequest {
                slot: place.clone(),
            });
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum Offered {
    TakeOff,
    Carried {
        verb: Option<&'static str>,
        bound: bool,
    },
}

impl Offered {
    fn of(world: &World, item: ItemId, selected: &Selected) -> Offered {
        let def = item.get(world.resource::<Content>());
        match selected {
            Selected::Worn(_) => Offered::TakeOff,
            Selected::Slot { .. } => Offered::Carried {
                verb: match def.kind {
                    ItemKind::Equipment { .. } => Some("Equip"),
                    ItemKind::Usable { .. } => Some("Use"),
                    ItemKind::Resource => None,
                },
                bound: def.bound(),
            },
        }
    }

    fn show(self, ui: &mut Ui) -> Option<Press> {
        match self {
            Offered::TakeOff => controls::primary(ui, "Take off")
                .clicked()
                .then_some(Press::TakeOff),
            Offered::Carried { verb, bound } => {
                let used = verb.is_some_and(|verb| controls::primary(ui, verb).clicked());
                let drop = ui
                    .add_enabled_ui(!bound, |ui| controls::button(ui, "Drop"))
                    .inner;
                if bound {
                    float::tooltip(&drop, |ui| {
                        ui.label(TextRole::Tooltip.rich("Can't be sold or dropped"));
                    });
                }
                match (used, drop.clicked()) {
                    (true, _) => Some(Press::Use),
                    (_, true) => Some(Press::Drop),
                    _ => None,
                }
            }
        }
    }
}

fn notes(world: &World, stack: ItemStack, selected: &Selected) -> Vec<ItemCardNote> {
    let compared = match selected {
        Selected::Slot { .. } => comparison(world, stack.item),
        Selected::Worn(_) => Vec::new(),
    };
    let noted = slot_notes(world, stack)
        .into_iter()
        .map(|text| ItemCardNote {
            text,
            tone: CardNoteTone::Plain,
        });
    compared.into_iter().chain(noted).collect()
}

fn comparison(world: &World, item: ItemId) -> Vec<ItemCardNote> {
    let content = world.resource::<Content>();
    let Some(me) = session::me(world) else {
        return Vec::new();
    };
    let displaced = equipment::displaced_by(world, me, item);
    if displaced.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = displaced
        .iter()
        .map(|worn| worn.get(content).name.clone())
        .collect();
    let than = words::join_and(&names);
    let gives = status::boosts_of(content, item);
    let gave: Vec<(StatKind, Option<f32>)> = displaced
        .iter()
        .flat_map(|worn| status::boosts_of(content, *worn))
        .collect();
    let mut stats: Vec<StatKind> = Vec::new();
    for (stat, _) in gives.iter().chain(&gave) {
        if !stats.contains(stat) {
            stats.push(*stat);
        }
    }
    stats
        .into_iter()
        .filter_map(|stat| {
            let total = |boosts: &[(StatKind, Option<f32>)]| {
                boosts
                    .iter()
                    .filter(|(boosted, _)| *boosted == stat)
                    .map(|(_, by)| *by)
                    .sum::<Option<f32>>()
            };
            let change = total(&gives)? - total(&gave)?;
            let name: &'static str = stat.into();
            if change > 0.0 {
                Some(ItemCardNote {
                    text: format!("{change} more {name} than {than}"),
                    tone: CardNoteTone::Strong,
                })
            } else if change < 0.0 {
                Some(ItemCardNote {
                    text: format!("{} less {name} than {than}", -change),
                    tone: CardNoteTone::Warning,
                })
            } else {
                None
            }
        })
        .collect()
}
