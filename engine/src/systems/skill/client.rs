use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::egui_wants_any_keyboard_input;

use super::{aim, bar, card, prompt, window};
use crate::core::gui::Band;
use crate::systems::input::map::InputAction;
use crate::systems::item::widget::{SlotAction, slot_action};
use crate::systems::screen::ClientScreen;

/// The client's side of skills: the tree window's plan, the hotbar's keys and drags, aiming, the
/// pinned card and the reset prompt.
pub struct SkillClientPlugin;

impl Plugin for SkillClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(card::SkillCardPlugin)
            .init_resource::<window::Plan>()
            .init_resource::<window::TreeView>()
            .init_resource::<bar::BarState>()
            .init_resource::<aim::Aiming>()
            .init_resource::<prompt::PromptState>()
            .add_systems(
                Update,
                (
                    bar::capture_key,
                    bar::keys.run_if(not(egui_wants_any_keyboard_input)),
                    window::settle_plan,
                    aim::settle,
                    aim::surface,
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (
                    aim::draw_aim.in_set(Band::World),
                    prompt::draw_prompt.in_set(Band::Modal),
                    (bar::drop_off_bar, bar::draw_carried)
                        .chain()
                        .in_set(Band::Pointer),
                )
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                OnExit(ClientScreen::Area),
                (
                    window::forget_plan,
                    bar::forget,
                    aim::forget,
                    prompt::forget,
                ),
            );
        slot_action(
            app,
            SlotAction {
                action: InputAction::PinToHotbar,
                active: |_| true,
                act: bar::pin_bag_slot,
                verb: "put it on the hotbar",
            },
        );
    }
}
