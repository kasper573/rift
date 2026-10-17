use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::helpers::bevy_to_egui_physical_key;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::{Content, ContentRow, Fixture, Loading, RowCheck};
use crate::core::text::{InputRef, KeyNames, TextPiece};

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    IntoStaticStr,
    VariantArray,
)]
#[strum(serialize_all = "snake_case")]
pub enum InputAction {
    Interact,
    Select,
    InspectItem,
    UseItem,
    DropItem,
    SellItem,
    CarryItem,
    ToggleInventory,
    ToggleEquipment,
    ToggleStats,
    ToggleQuestLog,
    ToggleSettings,
    ToggleTerminal,
    Dismiss,
    SubmitText,
    TakeDefault,
    Respawn,
    ChoosePrevious,
    ChooseNext,
    PickChoice,
    Advance,
    Choice1,
    Choice2,
    Choice3,
    Choice4,
    Choice5,
    Choice6,
    Choice7,
    Choice8,
    Choice9,
    ToggleHistory,
    ToggleSkills,
    Hotbar1,
    Hotbar2,
    Hotbar3,
    Hotbar4,
    Hotbar5,
    Hotbar6,
    Hotbar7,
    Hotbar8,
    Hotbar9,
    PinToHotbar,
}

impl Fixture for InputAction {
    type Row = InputDef;
}

impl InputAction {
    pub fn gestures(self) -> &'static [GestureKind] {
        match self {
            InputAction::Advance | InputAction::PickChoice => {
                &[GestureKind::Press, GestureKind::Click]
            }
            InputAction::Select
            | InputAction::InspectItem
            | InputAction::UseItem
            | InputAction::DropItem
            | InputAction::SellItem
            | InputAction::PinToHotbar => &[GestureKind::Click],
            InputAction::CarryItem => &[GestureKind::Drag],
            InputAction::SubmitText | InputAction::TakeDefault => &[GestureKind::Shortcut],
            _ => &[GestureKind::Press],
        }
    }
}

/// The ways the game reads an action. A binding the action's reader never sees is rejected at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Press,
    Click,
    Drag,
    Shortcut,
}

impl GestureKind {
    fn accepts(self, binding: InputBinding) -> bool {
        match self {
            GestureKind::Press => matches!(
                binding,
                InputBinding::Key(_) | InputBinding::AnyKey | InputBinding::Mouse { .. }
            ),
            GestureKind::Click => matches!(binding, InputBinding::Click(_)),
            GestureKind::Drag => matches!(binding, InputBinding::Drag(_)),
            GestureKind::Shortcut => matches!(binding, InputBinding::Key(_)),
        }
    }
}

pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    for (id, def) in loading.rows::<InputDef>(&content) {
        let key = id.key(&content);
        let Some(action) = InputAction::VARIANTS
            .iter()
            .copied()
            .find(|action| action.key() == key)
        else {
            continue;
        };
        let mut check = RowCheck::default();
        for (index, binding) in def.bindings.iter().enumerate() {
            if !action
                .gestures()
                .iter()
                .any(|gesture| gesture.accepts(*binding))
            {
                check.error(
                    format!("/bindings/{index}"),
                    "unread binding",
                    binding.name(),
                    "a binding the game reads for this action",
                );
            }
        }
        loading.row_errors(id, check);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InputDef {
    pub label: String,
    pub bindings: Vec<InputBinding>,
}

impl ContentRow for InputDef {
    const TABLE: &'static str = "input";
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputBinding {
    Key(KeyGesture),
    AnyKey,
    Mouse {
        #[schemars(schema_with = "mouse_button_schema")]
        button: MouseButton,
        #[serde(flatten)]
        modifiers: KeyModifiers,
    },
    Click(ClickGesture),
    Drag(DragGesture),
}

impl InputBinding {
    pub const fn key(key: KeyCode) -> InputBinding {
        InputBinding::Key(KeyGesture {
            key,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn mouse(button: MouseButton) -> InputBinding {
        InputBinding::Mouse {
            button,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub const fn click(button: MouseButton) -> InputBinding {
        InputBinding::Click(ClickGesture {
            button,
            times: 1,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn drag(button: MouseButton) -> InputBinding {
        InputBinding::Drag(DragGesture {
            button,
            modifiers: KeyModifiers::NONE,
        })
    }

    pub const fn with(self, held: KeyModifiers) -> InputBinding {
        match self {
            InputBinding::Key(gesture) => InputBinding::Key(KeyGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
            InputBinding::AnyKey => InputBinding::AnyKey,
            InputBinding::Mouse { button, modifiers } => InputBinding::Mouse {
                button,
                modifiers: modifiers.and(held),
            },
            InputBinding::Click(gesture) => InputBinding::Click(ClickGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
            InputBinding::Drag(gesture) => InputBinding::Drag(DragGesture {
                modifiers: gesture.modifiers.and(held),
                ..gesture
            }),
        }
    }

    pub fn name(&self) -> String {
        let (modifiers, name) = match *self {
            InputBinding::Key(gesture) => (gesture.modifiers, key_name(gesture.key)),
            InputBinding::AnyKey => (KeyModifiers::NONE, "any key".to_owned()),
            InputBinding::Mouse { button, modifiers } => (modifiers, click_name(button, 1)),
            InputBinding::Click(gesture) => {
                (gesture.modifiers, click_name(gesture.button, gesture.times))
            }
            InputBinding::Drag(gesture) => (gesture.modifiers, drag_name(gesture.button)),
        };
        capitalized(format!("{}{name}", held_prefix(modifiers)))
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct InputMap(HashMap<InputAction, Vec<InputBinding>>);

impl InputMap {
    pub fn new(content: &Content) -> InputMap {
        InputMap(
            InputAction::VARIANTS
                .iter()
                .map(|&action| (action, action.get(content).bindings.to_vec()))
                .collect(),
        )
    }

    pub fn bindings(&self, action: InputAction) -> &[InputBinding] {
        self.0.get(&action).map_or(&[], Vec::as_slice)
    }

    pub fn name(&self, action: InputAction) -> Option<String> {
        self.bindings(action).first().map(InputBinding::name)
    }

    pub fn just_pressed(
        &self,
        action: InputAction,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.bindings(action).iter().any(|binding| match *binding {
            InputBinding::Key(gesture) => gesture.just_pressed(keys),
            InputBinding::AnyKey => keys.get_just_pressed().next().is_some(),
            InputBinding::Mouse { button, modifiers } => {
                mouse.just_pressed(button) && KeyModifiers::held(keys) == modifiers
            }
            InputBinding::Click(_) | InputBinding::Drag(_) => false,
        })
    }

    pub fn pressed(
        &self,
        action: InputAction,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.bindings(action).iter().any(|binding| match *binding {
            InputBinding::Key(gesture) => gesture.pressed(keys),
            InputBinding::AnyKey => keys.get_pressed().next().is_some(),
            InputBinding::Mouse { button, modifiers } => {
                mouse.pressed(button) && KeyModifiers::held(keys) == modifiers
            }
            InputBinding::Click(_) | InputBinding::Drag(_) => false,
        })
    }

    pub fn clicked(
        &self,
        action: InputAction,
        response: &egui::Response,
        keys: &ButtonInput<KeyCode>,
    ) -> bool {
        self.bindings(action).iter().any(|binding| match binding {
            InputBinding::Click(gesture) => gesture.clicked(response, keys),
            _ => false,
        })
    }

    pub fn drag_started(
        &self,
        action: InputAction,
        response: &egui::Response,
        keys: &ButtonInput<KeyCode>,
    ) -> bool {
        self.bindings(action).iter().any(|binding| match binding {
            InputBinding::Drag(gesture) => gesture.started(response, keys),
            _ => false,
        })
    }

    pub fn shortcut(&self, action: InputAction) -> Option<egui::KeyboardShortcut> {
        self.bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Key(gesture) => Some(egui::KeyboardShortcut::new(
                    gesture.modifiers.egui(),
                    bevy_to_egui_physical_key(&gesture.key)?,
                )),
                _ => None,
            })
    }

    pub fn just_pressed_index(
        &self,
        actions: &[InputAction],
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> Option<usize> {
        actions
            .iter()
            .position(|action| self.just_pressed(*action, keys, mouse))
    }

    fn key_names(&self) -> KeyNames {
        KeyNames(
            InputAction::VARIANTS
                .iter()
                .map(|&action| (action.into(), self.name(action).unwrap_or_default()))
                .collect(),
        )
    }
}

#[derive(SystemParam)]
pub struct ActionInput<'w> {
    map: Res<'w, InputMap>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
}

impl ActionInput<'_> {
    pub fn just_pressed(&self, action: InputAction) -> bool {
        self.map.just_pressed(action, &self.keys, &self.mouse)
    }

    pub fn clicked(&self, action: InputAction, response: &egui::Response) -> bool {
        self.map.clicked(action, response, &self.keys)
    }
}

pub fn just_pressed(world: &World, action: InputAction) -> bool {
    world.resource::<InputMap>().just_pressed(
        action,
        world.resource::<ButtonInput<KeyCode>>(),
        world.resource::<ButtonInput<MouseButton>>(),
    )
}

pub fn just_pressed_index(world: &World, actions: &[InputAction]) -> Option<usize> {
    world.resource::<InputMap>().just_pressed_index(
        actions,
        world.resource::<ButtonInput<KeyCode>>(),
        world.resource::<ButtonInput<MouseButton>>(),
    )
}

pub fn pressed(world: &World, action: InputAction) -> bool {
    world.resource::<InputMap>().pressed(
        action,
        world.resource::<ButtonInput<KeyCode>>(),
        world.resource::<ButtonInput<MouseButton>>(),
    )
}

pub fn clicked(world: &World, action: InputAction, response: &egui::Response) -> bool {
    world
        .resource::<InputMap>()
        .clicked(action, response, world.resource::<ButtonInput<KeyCode>>())
}

pub fn drag_started(world: &World, action: InputAction, response: &egui::Response) -> bool {
    world.resource::<InputMap>().drag_started(
        action,
        response,
        world.resource::<ButtonInput<KeyCode>>(),
    )
}

pub fn shortcut(world: &World, action: InputAction) -> Option<egui::KeyboardShortcut> {
    world.resource::<InputMap>().shortcut(action)
}

pub fn input(action: InputAction) -> TextPiece {
    TextPiece::Key(action.into())
}

impl From<InputAction> for InputRef {
    fn from(action: InputAction) -> InputRef {
        InputRef(action as u16)
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PreUpdate,
        publish.run_if(resource_exists_and_changed::<InputMap>),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct KeyGesture {
    #[schemars(schema_with = "key_code_schema")]
    pub key: KeyCode,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

impl KeyGesture {
    pub fn just_pressed(&self, keys: &ButtonInput<KeyCode>) -> bool {
        keys.just_pressed(self.key) && KeyModifiers::held(keys) == self.modifiers
    }

    pub fn pressed(&self, keys: &ButtonInput<KeyCode>) -> bool {
        keys.pressed(self.key) && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ClickGesture {
    #[schemars(schema_with = "mouse_button_schema")]
    pub button: MouseButton,
    #[schemars(extend("x-count" = true))]
    pub times: u8,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

impl ClickGesture {
    fn clicked(&self, response: &egui::Response, keys: &ButtonInput<KeyCode>) -> bool {
        let Some(button) = pointer_button(self.button) else {
            return false;
        };
        let clicked = match self.times {
            0 | 1 => response.clicked_by(button),
            2 => response.double_clicked_by(button),
            _ => response.triple_clicked_by(button),
        };
        clicked && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct DragGesture {
    #[schemars(schema_with = "mouse_button_schema")]
    pub button: MouseButton,
    #[serde(flatten)]
    pub modifiers: KeyModifiers,
}

impl DragGesture {
    fn started(&self, response: &egui::Response, keys: &ButtonInput<KeyCode>) -> bool {
        pointer_button(self.button).is_some_and(|button| response.drag_started_by(button))
            && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct KeyModifiers {
    #[serde(skip_serializing_if = "is_false")]
    pub ctrl: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub shift: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub alt: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub super_key: bool,
}

impl KeyModifiers {
    pub const NONE: KeyModifiers = KeyModifiers {
        ctrl: false,
        shift: false,
        alt: false,
        super_key: false,
    };

    pub const fn and(self, other: KeyModifiers) -> KeyModifiers {
        KeyModifiers {
            ctrl: self.ctrl || other.ctrl,
            shift: self.shift || other.shift,
            alt: self.alt || other.alt,
            super_key: self.super_key || other.super_key,
        }
    }

    pub fn held(keys: &ButtonInput<KeyCode>) -> KeyModifiers {
        KeyModifiers {
            ctrl: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
            shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            alt: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
            super_key: keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]),
        }
    }

    fn egui(self) -> egui::Modifiers {
        egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: self.super_key,
            command: self.ctrl,
        }
    }
}

pub fn key_code_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "$comment": "bevy_input::keyboard::KeyCode", "type": ["string", "object"] })
}

pub fn mouse_button_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "$comment": "bevy_input::mouse::MouseButton", "type": ["string", "object"] })
}

fn publish(map: Res<InputMap>, mut names: ResMut<KeyNames>) {
    *names = map.key_names();
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

fn pointer_button(button: MouseButton) -> Option<egui::PointerButton> {
    match button {
        MouseButton::Left => Some(egui::PointerButton::Primary),
        MouseButton::Right => Some(egui::PointerButton::Secondary),
        MouseButton::Middle => Some(egui::PointerButton::Middle),
        _ => None,
    }
}

fn held_prefix(modifiers: KeyModifiers) -> String {
    [
        (modifiers.ctrl, "Ctrl+"),
        (modifiers.shift, "Shift+"),
        (modifiers.alt, "Alt+"),
        (modifiers.super_key, "Super+"),
    ]
    .into_iter()
    .filter_map(|(held, prefix)| held.then_some(prefix))
    .collect()
}

fn key_name(key: KeyCode) -> String {
    let named = match key {
        KeyCode::Escape => "Esc",
        KeyCode::ArrowUp => "↑",
        KeyCode::ArrowDown => "↓",
        KeyCode::ArrowLeft => "←",
        KeyCode::ArrowRight => "→",
        _ => {
            let debug = format!("{key:?}");
            return ["Key", "Digit"]
                .iter()
                .find_map(|prefix| debug.strip_prefix(prefix))
                .unwrap_or(&debug)
                .to_owned();
        }
    };
    named.to_owned()
}

fn click_name(button: MouseButton, times: u8) -> String {
    let press = match times {
        1 => "",
        2 => "double-",
        _ => "multi-",
    };
    format!("{press}{}click", button_prefix(button))
}

fn drag_name(button: MouseButton) -> String {
    format!("{}drag", button_prefix(button))
}

fn button_prefix(button: MouseButton) -> String {
    match button {
        MouseButton::Left => String::new(),
        other => format!("{other:?}-").to_lowercase(),
    }
}

fn capitalized(name: String) -> String {
    let mut letters = name.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}
