use bevy::prelude::*;
use bevy_egui::EguiPreUpdateSet;
use bevy_egui::input::EguiWantsInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::map::InputAction;
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::hud;

/// Where the player is this frame, innermost first: the place that decides which actions are
/// read.
#[derive(
    Resource, Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum InputScope {
    /// The interface has the keyboard: a text field, or a list that is open.
    Typing,
    /// A modal is up.
    Modal,
    /// Talking, with or without a panel docked.
    Conversation,
    Menu,
    #[default]
    World,
}

impl InputAction {
    /// Where the action is read as a key or a held button. A click or a drag on a widget is read
    /// wherever the widget is drawn.
    pub fn scopes(self) -> &'static [InputScope] {
        use InputScope::{Conversation, Menu, Modal, Typing, World};
        match self {
            InputAction::SubmitText | InputAction::CompleteText => &[Typing],
            InputAction::TakeDefault => &[Modal],
            InputAction::Advance
            | InputAction::PickChoice
            | InputAction::ChoosePrevious
            | InputAction::ChooseNext
            | InputAction::Choice1
            | InputAction::Choice2
            | InputAction::Choice3
            | InputAction::Choice4
            | InputAction::Choice5
            | InputAction::Choice6
            | InputAction::Choice7
            | InputAction::Choice8
            | InputAction::Choice9 => &[Conversation],
            InputAction::Respawn
            | InputAction::TurnLeft
            | InputAction::TurnRight
            | InputAction::TiltUp
            | InputAction::TiltDown
            | InputAction::CameraHome
            | InputAction::Emotes => &[World],
            InputAction::OpenCharacter
            | InputAction::OpenSkills
            | InputAction::OpenQuests
            | InputAction::OpenHistory
            | InputAction::OpenSettings
            | InputAction::Hotbar1
            | InputAction::Hotbar2
            | InputAction::Hotbar3
            | InputAction::Hotbar4
            | InputAction::Hotbar5
            | InputAction::Hotbar6
            | InputAction::Hotbar7
            | InputAction::Hotbar8
            | InputAction::Hotbar9
            | InputAction::FocusChat => &[Menu, World],
            InputAction::Dismiss
            | InputAction::Interact
            | InputAction::TurnCamera
            | InputAction::TiltCamera
            | InputAction::Select
            | InputAction::InspectItem
            | InputAction::UseItem
            | InputAction::DropItem
            | InputAction::SellItem
            | InputAction::PlaceItem
            | InputAction::PlaceStack
            | InputAction::CarryItem
            | InputAction::PinToHotbar
            | InputAction::RaiseRank
            | InputAction::LowerRank
            | InputAction::InspectSkill
            | InputAction::UseSlot
            | InputAction::SlotOptions => &[Typing, Modal, Conversation, Menu, World],
        }
    }
}

/// Whether the place the player is in reads `action`.
pub fn reads(world: &World, action: InputAction) -> bool {
    action.scopes().contains(world.resource::<InputScope>())
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InputScope>()
        .add_systems(PreUpdate, decide.before(EguiPreUpdateSet::BeginPass));
}

// Before the interface begins its pass, where a key may hand the keyboard on: a press is read in
// the place the player was in when they made it, so one press never takes two steps.
fn decide(world: &mut World) {
    let scope = if typing(world) {
        InputScope::Typing
    } else if hud::confirming(world) {
        InputScope::Modal
    } else if stage::leavable(world) {
        InputScope::Conversation
    } else if hud::menu::is_open(world) {
        InputScope::Menu
    } else {
        InputScope::World
    };
    world.resource_mut::<InputScope>().set_if_neq(scope);
}

fn typing(world: &World) -> bool {
    let list_open = world
        .get_resource::<EguiWantsInput>()
        .is_some_and(EguiWantsInput::is_popup_open);
    list_open || gui::context(world).is_some_and(|gui| gui.text_edit_focused())
}
