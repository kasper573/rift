use bevy::prelude::*;
use ui::egui::{self, Pos2, Rect, Ui, UiBuilder};
use ui::{face, sheet};

use crate::systems::actor::Name;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::widget::{InventoryPane, inventory_pane, pane_width};
use crate::systems::player::session;
use crate::systems::{equipment, stat};

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
        let worn = Rect::from_min_max(
            room.min,
            Pos2::new(
                room.min.x + equipment::widget::PANE_WIDTH + SCROLL_BAR,
                room.max.y,
            ),
        );
        let carried = Rect::from_min_max(
            Pos2::new(worn.max.x + COLUMN_GAP, room.min.y),
            Pos2::new(
                worn.max.x + COLUMN_GAP + pane_width(INVENTORY_COLUMNS),
                room.max.y,
            ),
        );
        column(ui, "worn", worn, |ui| {
            if let Some(name) = session::my_character(world).and_then(|me| me.get::<Name>()) {
                ui.label(face::bold(name.name.as_str()));
                ui.add_space(4.0);
            }
            sheet::section(ui, "Equipment");
            equipment::widget::pane(ui, world);
            ui.add_space(SECTION_GAP);
            sheet::section(ui, "Stats");
            stat::widget::pane(ui, world);
        });
        column(ui, "carried", carried, |ui| {
            let mut selected = world.get_resource_or_init::<SelectedSlot>().0;
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: Some("Inventory"),
                selected: &mut selected,
            };
            inventory_pane(ui, world, pane);
            world.resource_mut::<SelectedSlot>().0 = selected;
        });
    }
}

const COLUMN_GAP: f32 = 16.0;
const SECTION_GAP: f32 = 10.0;
const INVENTORY_COLUMNS: u32 = 6;
const SCROLL_BAR: f32 = 14.0;

#[derive(Resource, Default)]
struct SelectedSlot(Option<u32>);

fn column(ui: &mut Ui, id: &str, rect: Rect, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        let area = egui::ScrollArea::vertical()
            .id_salt(("character", id))
            .auto_shrink(false);
        sheet::scroll(ui, area, add);
    });
}
