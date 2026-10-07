use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use ui::KeyModifiers;

use crate::systems::input::map::{InputBinding, InputDef};

crate::table! {
    #![expose]
    Interact: InputDef {
        label: "Walk, attack, talk or pick up",
        bindings: &[InputBinding::mouse(MouseButton::Left)],
    },
    Select: InputDef {
        label: "Select a row or tick a box",
        bindings: &[InputBinding::click(MouseButton::Left)],
    },
    InspectItem: InputDef {
        label: "Show an item's card",
        bindings: &[InputBinding::click(MouseButton::Right)],
    },
    UseItem: InputDef {
        label: "Use, wear or take off an item",
        bindings: &[InputBinding::double_click(MouseButton::Left)],
    },
    DropItem: InputDef {
        label: "Drop an item from your bag",
        bindings: &[InputBinding::click(MouseButton::Left).with(KeyModifiers::CTRL)],
    },
    SellItem: InputDef {
        label: "Sell an item to the open shop",
        bindings: &[InputBinding::click(MouseButton::Right)],
    },
    CarryItem: InputDef {
        label: "Carry an item out of your bag",
        bindings: &[InputBinding::drag(MouseButton::Left)],
    },
    ToggleInventory: InputDef {
        label: "Open or close the inventory",
        bindings: &[InputBinding::key(KeyCode::KeyI)],
    },
    ToggleEquipment: InputDef {
        label: "Open or close the equipment",
        bindings: &[InputBinding::key(KeyCode::KeyE)],
    },
    ToggleStats: InputDef {
        label: "Open or close the stats",
        bindings: &[InputBinding::key(KeyCode::KeyK)],
    },
    ToggleQuestLog: InputDef {
        label: "Open or close the quest log",
        bindings: &[InputBinding::key(KeyCode::KeyL)],
    },
    ToggleSettings: InputDef {
        label: "Open or close the settings",
        bindings: &[InputBinding::key(KeyCode::KeyO)],
    },
    ToggleTerminal: InputDef {
        label: "Open or close the terminal",
        bindings: &[InputBinding::key(KeyCode::KeyC)],
    },
    Dismiss: InputDef {
        label: "Close what's on top, stop typing, or leave the conversation",
        bindings: &[InputBinding::key(KeyCode::Escape)],
    },
    SubmitText: InputDef {
        label: "Send what you typed",
        bindings: &[InputBinding::key(KeyCode::Enter)],
    },
    TakeDefault: InputDef {
        label: "Give a question its default answer",
        bindings: &[InputBinding::key(KeyCode::Enter)],
    },
    Respawn: InputDef {
        label: "Get back up after dying",
        bindings: &[InputBinding::AnyKey],
    },
    ChoosePrevious: InputDef {
        label: "Move to the previous choice",
        bindings: &[InputBinding::key(KeyCode::ArrowUp)],
    },
    ChooseNext: InputDef {
        label: "Move to the next choice",
        bindings: &[InputBinding::key(KeyCode::ArrowDown)],
    },
    PickChoice: InputDef {
        label: "Pick the selected choice, or the one clicked",
        bindings: &[InputBinding::key(KeyCode::Enter), InputBinding::click(MouseButton::Left)],
    },
    Advance: InputDef {
        label: "Finish or continue a line",
        bindings: &[
            InputBinding::key(KeyCode::Space),
            InputBinding::key(KeyCode::Enter),
            InputBinding::click(MouseButton::Left),
        ],
    },
    Choice1: InputDef {
        label: "Pick the first choice",
        bindings: &[InputBinding::key(KeyCode::Digit1)],
    },
    Choice2: InputDef {
        label: "Pick the second choice",
        bindings: &[InputBinding::key(KeyCode::Digit2)],
    },
    Choice3: InputDef {
        label: "Pick the third choice",
        bindings: &[InputBinding::key(KeyCode::Digit3)],
    },
    Choice4: InputDef {
        label: "Pick the fourth choice",
        bindings: &[InputBinding::key(KeyCode::Digit4)],
    },
    Choice5: InputDef {
        label: "Pick the fifth choice",
        bindings: &[InputBinding::key(KeyCode::Digit5)],
    },
    Choice6: InputDef {
        label: "Pick the sixth choice",
        bindings: &[InputBinding::key(KeyCode::Digit6)],
    },
    Choice7: InputDef {
        label: "Pick the seventh choice",
        bindings: &[InputBinding::key(KeyCode::Digit7)],
    },
    Choice8: InputDef {
        label: "Pick the eighth choice",
        bindings: &[InputBinding::key(KeyCode::Digit8)],
    },
    Choice9: InputDef {
        label: "Pick the ninth choice",
        bindings: &[InputBinding::key(KeyCode::Digit9)],
    },
    ToggleHistory: InputDef {
        label: "Show or hide the conversation so far",
        bindings: &[InputBinding::key(KeyCode::KeyH)],
    },
    SpectatePrevious: InputDef {
        label: "Watch the previous player",
        bindings: &[InputBinding::key(KeyCode::ArrowLeft)],
    },
    SpectateNext: InputDef {
        label: "Watch the next player",
        bindings: &[InputBinding::key(KeyCode::ArrowRight)],
    },
    CycleDebugView: InputDef {
        label: "Cycle the debug overlays",
        bindings: &[InputBinding::key(KeyCode::F1)],
    },
    ToggleHitboxes: InputDef {
        label: "Show or hide hitboxes",
        bindings: &[InputBinding::key(KeyCode::F2)],
    },
}
