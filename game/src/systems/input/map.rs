use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{Click, Pointer};
use bevy::prelude::*;
use ui::{
    CatalogEntry, ClickGesture, DragGesture, InputCatalog, InputRef, KeyGesture, KeyModifiers,
    RichPiece,
};

pub use crate::data::input::Id as InputAction;

pub struct InputDef {
    pub label: &'static str,
    pub bindings: &'static [InputBinding],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputBinding {
    Key(KeyGesture),
    AnyKey,
    Mouse {
        button: MouseButton,
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

    pub const fn double_click(button: MouseButton) -> InputBinding {
        InputBinding::Click(ClickGesture {
            button,
            times: 2,
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

impl Default for InputMap {
    fn default() -> InputMap {
        InputMap(
            InputAction::VARIANTS
                .iter()
                .map(|&action| (action, action.get().bindings.to_vec()))
                .collect(),
        )
    }
}

impl InputMap {
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
        click: &Pointer<Click>,
        keys: &ButtonInput<KeyCode>,
    ) -> bool {
        self.bindings(action).iter().any(|binding| match binding {
            InputBinding::Click(gesture) => gesture.matches(click, keys),
            _ => false,
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

    fn catalog(&self) -> InputCatalog {
        InputCatalog(
            InputAction::VARIANTS
                .iter()
                .map(|&action| {
                    let bindings = self.bindings(action);
                    let entry = CatalogEntry {
                        name: self.name(action).unwrap_or_default(),
                        keys: bindings
                            .iter()
                            .filter_map(|binding| match binding {
                                InputBinding::Key(gesture) => Some(*gesture),
                                _ => None,
                            })
                            .collect(),
                        clicks: bindings
                            .iter()
                            .filter_map(|binding| match binding {
                                InputBinding::Click(gesture) => Some(*gesture),
                                _ => None,
                            })
                            .collect(),
                        drags: bindings
                            .iter()
                            .filter_map(|binding| match binding {
                                InputBinding::Drag(gesture) => Some(*gesture),
                                _ => None,
                            })
                            .collect(),
                    };
                    (action.into(), entry)
                })
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

    pub fn clicked(&self, action: InputAction, click: &Pointer<Click>) -> bool {
        self.map.clicked(action, click, &self.keys)
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

pub fn input(action: InputAction) -> RichPiece {
    RichPiece::Input(action.into())
}

impl From<InputAction> for InputRef {
    fn from(action: InputAction) -> InputRef {
        InputRef(action.index() as u16)
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InputMap>()
        .add_systems(PreUpdate, publish.run_if(resource_changed::<InputMap>));
}

fn publish(map: Res<InputMap>, mut catalog: ResMut<InputCatalog>) {
    *catalog = map.catalog();
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
