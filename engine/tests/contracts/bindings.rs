use bevy::prelude::*;
use engine::systems::input::map::{self, InputAction, InputBinding, InputMap, KeyModifiers};
use engine::systems::input::scope::InputScope;
use testkit::Game;

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
