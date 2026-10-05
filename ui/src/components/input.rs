use std::collections::HashMap;

use bevy_color::{Alpha, Color};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_input::ButtonInput;
use bevy_input::keyboard::KeyCode;
use bevy_input::mouse::MouseButton;
use bevy_picking::events::{Click, Pointer};
use bevy_picking::pointer::PointerButton;
use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn, template_value};
use bevy_text::{TextColor, TextFont};
use bevy_ui::widget::Text;
use bevy_ui::{
    AlignItems, BackgroundColor, BorderColor, BorderRadius, JustifyContent, Node, UiRect, Val,
};

use crate::component;
use crate::components::text::font;
use crate::tokens::{radius, typography};

const CAP_SCALE: f32 = 0.85;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputRef(pub u16);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogEntry {
    pub name: String,
    pub keys: Vec<KeyGesture>,
    pub clicks: Vec<ClickGesture>,
    pub drags: Vec<DragGesture>,
}

#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct InputCatalog(pub HashMap<InputRef, CatalogEntry>);

impl InputCatalog {
    pub fn name(&self, input: InputRef) -> Option<&str> {
        self.0.get(&input).map(|entry| entry.name.as_str())
    }

    pub fn just_pressed(&self, input: InputRef, keys: &ButtonInput<KeyCode>) -> bool {
        self.0
            .get(&input)
            .is_some_and(|entry| entry.keys.iter().any(|gesture| gesture.just_pressed(keys)))
    }

    pub fn keyed(&self, input: InputRef, key: KeyCode, keys: &ButtonInput<KeyCode>) -> bool {
        self.0
            .get(&input)
            .is_some_and(|entry| entry.keys.iter().any(|gesture| gesture.matches(key, keys)))
    }

    pub fn clicked(
        &self,
        input: InputRef,
        click: &Pointer<Click>,
        keys: &ButtonInput<KeyCode>,
    ) -> bool {
        self.0.get(&input).is_some_and(|entry| {
            entry
                .clicks
                .iter()
                .any(|gesture| gesture.matches(click, keys))
        })
    }

    pub fn dragged(
        &self,
        input: InputRef,
        button: PointerButton,
        keys: &ButtonInput<KeyCode>,
    ) -> bool {
        self.0.get(&input).is_some_and(|entry| {
            entry
                .drags
                .iter()
                .any(|gesture| gesture.matches(button, keys))
        })
    }
}

#[derive(SystemParam)]
pub(crate) struct CatalogInput<'w> {
    catalog: Res<'w, InputCatalog>,
    keys: Res<'w, ButtonInput<KeyCode>>,
}

impl CatalogInput<'_> {
    pub(crate) fn keyed(&self, input: InputRef, key: KeyCode) -> bool {
        self.catalog.keyed(input, key, &self.keys)
    }

    pub(crate) fn clicked(&self, input: InputRef, click: &Pointer<Click>) -> bool {
        self.catalog.clicked(input, click, &self.keys)
    }

    pub(crate) fn dragged(&self, input: InputRef, button: PointerButton) -> bool {
        self.catalog.dragged(input, button, &self.keys)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyGesture {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyGesture {
    pub fn matches(&self, key: KeyCode, keys: &ButtonInput<KeyCode>) -> bool {
        self.key == key && KeyModifiers::held(keys) == self.modifiers
    }

    pub fn just_pressed(&self, keys: &ButtonInput<KeyCode>) -> bool {
        keys.just_pressed(self.key) && KeyModifiers::held(keys) == self.modifiers
    }

    pub fn pressed(&self, keys: &ButtonInput<KeyCode>) -> bool {
        keys.pressed(self.key) && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClickGesture {
    pub button: MouseButton,
    pub times: u8,
    pub modifiers: KeyModifiers,
}

impl ClickGesture {
    pub fn matches(&self, click: &Pointer<Click>, keys: &ButtonInput<KeyCode>) -> bool {
        pointer_button(self.button) == Some(click.button)
            && click.count.is_multiple_of(self.times.max(1))
            && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DragGesture {
    pub button: MouseButton,
    pub modifiers: KeyModifiers,
}

impl DragGesture {
    pub fn matches(&self, button: PointerButton, keys: &ButtonInput<KeyCode>) -> bool {
        pointer_button(self.button) == Some(button) && KeyModifiers::held(keys) == self.modifiers
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl KeyModifiers {
    pub const NONE: KeyModifiers = KeyModifiers {
        ctrl: false,
        shift: false,
        alt: false,
        super_key: false,
    };
    pub const CTRL: KeyModifiers = KeyModifiers {
        ctrl: true,
        ..KeyModifiers::NONE
    };
    pub const SHIFT: KeyModifiers = KeyModifiers {
        shift: true,
        ..KeyModifiers::NONE
    };
    pub const ALT: KeyModifiers = KeyModifiers {
        alt: true,
        ..KeyModifiers::NONE
    };
    pub const SUPER: KeyModifiers = KeyModifiers {
        super_key: true,
        ..KeyModifiers::NONE
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
}

#[derive(Component, Clone, Copy, Debug)]
#[require(Text)]
pub struct InputLabel(pub InputRef);

pub fn input_cap(input: InputRef, size: f32, color: Color) -> impl Scene {
    let node = Node {
        padding: UiRect::axes(Val::Px(size * 0.3), Val::Px(0.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(radius::S)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        min_width: Val::Px(size * 1.3),
        ..Default::default()
    };
    let label = TextFont {
        font_size: (size * CAP_SCALE).into(),
        ..font(typography::KEY)
    };
    bsn! {
        template_value(node)
        BackgroundColor({color.with_alpha(color.alpha() * 0.12)})
        component(BorderColor::all(color.with_alpha(color.alpha() * 0.5)))
        Pickable::IGNORE
        Children [
            (
                component(InputLabel(input))
                component(label)
                component(TextColor(color))
                Pickable::IGNORE
            ),
        ]
    }
}

pub(crate) fn name_inputs(
    catalog: Res<InputCatalog>,
    mut labels: Query<(Ref<InputLabel>, &mut Text)>,
) {
    for (label, mut text) in &mut labels {
        if !catalog.is_changed() && !label.is_added() {
            continue;
        }
        let name = catalog.name(label.0).unwrap_or_default();
        if text.0 != name {
            text.0 = name.to_owned();
        }
    }
}

fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Primary),
        MouseButton::Right => Some(PointerButton::Secondary),
        MouseButton::Middle => Some(PointerButton::Middle),
        _ => None,
    }
}
