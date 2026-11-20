use bevy::prelude::*;
use ui::egui::{self, Ui};
use ui::sheet;

use crate::core::content::Content;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::bag_view::{BagView, bag_view};
use crate::systems::item::widget::CarriedInputs;
use crate::systems::player::session;
use crate::systems::settings::Placed;
use crate::systems::stat;
use crate::systems::{equipment, messages};

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
    fn hints(&self, world: &World) -> Vec<(InputAction, String)> {
        let words = &messages::of(world.resource::<Content>()).player;
        let carried = CarriedInputs::of(world);
        let worn = session::me(world).is_some_and(|me| equipment::worn(world, me).next().is_some());
        [
            (
                InputAction::UseItem,
                words.use_wear_or_take_off(),
                carried.uses || worn,
            ),
            (
                InputAction::CarryItem,
                words.carry_or_pin(),
                carried.carries || worn,
            ),
            (InputAction::DropItem, words.drop(), carried.drops),
            (
                InputAction::InspectItem,
                words.open_card(),
                carried.inspects || worn,
            ),
        ]
        .into_iter()
        .filter(|(_, _, does)| *does)
        .map(|(action, does, _)| (action, does.to_owned()))
        .collect()
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let room = ui.available_rect_before_wrap();
        let widths = [STATS_WIDTH, equipment::widget::PANE_WIDTH];
        let ([stats, worn], carried) = sheet::columns(ui, room, widths);
        let content = world.resource::<Content>().clone();
        let words = messages::of(&content);
        column(ui, "stats", stats, |ui| {
            sheet::section(ui, words.player.stats());
            stat::widget::pane(ui, world);
        });
        column(ui, "worn", worn, |ui| {
            sheet::section(ui, words.player.equipment());
            equipment::widget::pane(ui, world);
        });
        let bag = egui::UiBuilder::new()
            .max_rect(carried.inner)
            .layout(egui::Layout::top_down(egui::Align::Min));
        ui.scope_builder(bag, |ui| {
            let view = BagView {
                heading: words.interface.inventory(),
                counts: true,
                placement: Placed::Menu,
            };
            bag_view(ui, world, view);
        });
    }
}

const STATS_WIDTH: f32 = 220.0;

fn column(ui: &mut Ui, id: &str, pane: sheet::Pane, add: impl FnOnce(&mut Ui)) {
    let area = egui::ScrollArea::vertical().id_salt(("character", id));
    sheet::scroll_pane(ui, pane, area, add);
}
