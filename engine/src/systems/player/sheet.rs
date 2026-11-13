use bevy::prelude::*;
use ui::egui::{self, Ui};
use ui::sheet;

use crate::systems::equipment;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::widget::{InventoryPane, inventory_pane};
use crate::systems::stat;

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
            (InputAction::UseItem, "use, wear or take off"),
            (InputAction::CarryItem, "carry, or put on the bar"),
            (InputAction::DropItem, "drop"),
            (InputAction::InspectItem, "open the card"),
        ]
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let room = ui.available_rect_before_wrap();
        let widths = [STATS_WIDTH, equipment::widget::PANE_WIDTH];
        let ([stats, worn], carried) = sheet::columns(ui, room, widths);
        column(ui, "stats", stats, |ui| {
            sheet::section(ui, "Stats");
            stat::widget::pane(ui, world);
        });
        column(ui, "worn", worn, |ui| {
            sheet::section(ui, "Equipment");
            equipment::widget::pane(ui, world);
        });
        column(ui, "carried", carried, |ui| {
            let pane = InventoryPane {
                width: ui.available_width(),
                heading: Some("Inventory"),
                selected: None,
            };
            inventory_pane(ui, world, pane);
        });
    }
}

const STATS_WIDTH: f32 = 220.0;

fn column(ui: &mut Ui, id: &str, pane: sheet::Pane, add: impl FnOnce(&mut Ui)) {
    let area = egui::ScrollArea::vertical().id_salt(("character", id));
    sheet::scroll_pane(ui, pane, area, add);
}
