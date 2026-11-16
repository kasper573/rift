use bevy::prelude::*;
use engine::systems::input::map::{
    self, ClickGesture, Clicks, InputAction, InputBinding, InputMap, KeyModifiers, WheelTurn,
};
use engine::systems::input::scope::InputScope;
use testkit::Game;
use ui::keys::{CapFace, MouseGlyph};

#[test]
fn a_modifier_bound_as_an_actions_own_key_stands_in_no_other_gestures_way() {
    let alt = KeyModifiers {
        alt: true,
        ..KeyModifiers::NONE
    };
    let content = Game::new()
        .bound(&[
            (InputAction::NameItems, InputBinding::key(KeyCode::AltLeft)),
            (
                InputAction::Interact,
                InputBinding::mouse(MouseButton::Left),
            ),
            (InputAction::CameraHome, InputBinding::key(KeyCode::KeyE)),
            (
                InputAction::OpenCharacter,
                InputBinding::key(KeyCode::KeyE).with(alt),
            ),
        ])
        .load();
    let pressed = |keys: &[KeyCode]| {
        let mut world = World::new();
        world.insert_resource(InputMap::new(&content));
        world.insert_resource(InputScope::World);
        let mut held = ButtonInput::<KeyCode>::default();
        for &key in keys {
            held.press(key);
        }
        world.insert_resource(held);
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Left);
        world.insert_resource(mouse);
        [
            InputAction::NameItems,
            InputAction::Interact,
            InputAction::CameraHome,
            InputAction::OpenCharacter,
        ]
        .map(|action| map::pressed(&world, action))
    };

    assert_eq!(
        pressed(&[KeyCode::AltLeft]),
        [true, true, false, false],
        "the key held alone leaves a click a click"
    );
    assert_eq!(
        pressed(&[KeyCode::AltLeft, KeyCode::KeyE]),
        [true, true, false, true],
        "a gesture that asks for the modifier still takes it as one"
    );
    assert_eq!(
        pressed(&[KeyCode::ShiftLeft, KeyCode::KeyE]),
        [false, false, false, false],
        "a modifier no action has as its key changes every gesture"
    );
    assert_eq!(
        pressed(&[KeyCode::KeyE]),
        [false, true, true, false],
        "nothing held beside a key"
    );
}

#[test]
fn a_binding_reads_as_a_key_cap_with_mouse_buttons_drawn_as_glyphs_and_named_in_words() {
    let held = KeyModifiers {
        ctrl: true,
        shift: true,
        alt: true,
        ..KeyModifiers::NONE
    };
    let clicked = |button, times| {
        InputBinding::Click(ClickGesture {
            button,
            times,
            modifiers: KeyModifiers::NONE,
        })
    };
    let mouse = |glyph, presses, drag| CapFace::Mouse {
        glyph,
        presses,
        drag,
    };
    let text = |words: &str| CapFace::Text(words.to_owned());
    let cases = [
        (
            InputBinding::key(KeyCode::KeyE),
            text("E"),
            "E",
            Some("Ctrl+Alt+Shift+E"),
        ),
        (
            InputBinding::key(KeyCode::Escape),
            text("Esc"),
            "Esc",
            Some("Ctrl+Alt+Shift+Esc"),
        ),
        (InputBinding::AnyKey, text("Any key"), "Any key", None),
        (
            InputBinding::mouse(MouseButton::Right),
            mouse(MouseGlyph::Right, 1, false),
            "Right-click",
            Some("Ctrl+Alt+Shift+right-click"),
        ),
        (
            InputBinding::click(MouseButton::Left),
            mouse(MouseGlyph::Left, 1, false),
            "Click",
            Some("Ctrl+Alt+Shift+click"),
        ),
        (
            clicked(MouseButton::Left, Clicks::Double),
            mouse(MouseGlyph::Left, 2, false),
            "Double-click",
            Some("Ctrl+Alt+Shift+double-click"),
        ),
        (
            clicked(MouseButton::Middle, Clicks::Triple),
            mouse(MouseGlyph::Middle, 3, false),
            "Triple-middle-click",
            Some("Ctrl+Alt+Shift+triple-middle-click"),
        ),
        (
            InputBinding::drag(MouseButton::Left),
            mouse(MouseGlyph::Left, 1, true),
            "Drag",
            Some("Ctrl+Alt+Shift+drag"),
        ),
        (
            InputBinding::drag(MouseButton::Right),
            mouse(MouseGlyph::Right, 1, true),
            "Right-drag",
            Some("Ctrl+Alt+Shift+right-drag"),
        ),
        (
            InputBinding::wheel(WheelTurn::Up),
            mouse(MouseGlyph::WheelUp, 1, false),
            "Wheel up",
            Some("Ctrl+Alt+Shift+wheel up"),
        ),
        (
            InputBinding::wheel(WheelTurn::Down),
            mouse(MouseGlyph::WheelDown, 1, false),
            "Wheel down",
            Some("Ctrl+Alt+Shift+wheel down"),
        ),
        (
            InputBinding::click(MouseButton::Back),
            mouse(MouseGlyph::Back, 1, false),
            "Mouse back",
            Some("Ctrl+Alt+Shift+mouse back"),
        ),
        (
            clicked(MouseButton::Forward, Clicks::Double),
            mouse(MouseGlyph::Forward, 2, false),
            "Mouse forward double-click",
            Some("Ctrl+Alt+Shift+mouse forward double-click"),
        ),
        (
            InputBinding::mouse(MouseButton::Other(6)),
            text("Mouse 6"),
            "Mouse 6",
            Some("Ctrl+Alt+Shift+mouse 6"),
        ),
    ];
    for (binding, face, name, held_name) in cases {
        let cap = binding.cap();
        assert_eq!(
            (&cap.held, &cap.face, cap.name.as_str(), binding.name()),
            (&Vec::<String>::new(), &face, name, name.to_owned()),
            "{name}"
        );
        let Some(held_name) = held_name else {
            continue;
        };
        let with = binding.with(held).cap();
        assert_eq!(
            (with.held, with.face, with.name),
            (
                ["Ctrl", "Alt", "Shift"].map(str::to_owned).to_vec(),
                face,
                held_name.to_owned()
            ),
            "{name}, held: the modifiers come first, always in the same order"
        );
    }
}
