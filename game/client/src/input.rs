use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use world::session;

use crate::Screen;
use crate::gestures;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        gestures::init(app);
        app.init_resource::<Latched>();
        app.add_systems(Update, touch_as_mouse);
        app.add_systems(Update, drive_intents.run_if(in_state(Screen::Playing)));
        app.add_systems(Update, respawn_when_dead.run_if(in_state(Screen::Playing)));
    }
}

/// Which active intent (if any) owns the press in progress. This is all the input layer is: pick the
/// first intent to claim a press, drive it until release; the intents live in `gestures/`.
#[derive(Resource, Default)]
struct Latched(Option<usize>);

fn drive_intents(world: &mut World) {
    let mut active = world.resource::<Latched>().0;
    let (pressed, just) = {
        let buttons = world.resource::<ButtonInput<MouseButton>>();
        (
            buttons.pressed(MouseButton::Left),
            buttons.just_pressed(MouseButton::Left),
        )
    };
    if !pressed {
        active = None;
    } else if just {
        active = gestures::ALL.iter().position(|intent| intent.claims(world));
        if let Some(index) = active {
            gestures::ALL[index].drive(world, true);
        }
    } else if let Some(index) = active {
        gestures::ALL[index].drive(world, false);
    }
    world.resource_mut::<Latched>().0 = active;
}

/// Bridges touch to the mouse so taps act as left-clicks for both the map and the UI, without the
/// rest of the game (or bevy's picking) needing to know about touch: the first active finger drives
/// the cursor, and a touch beginning/ending becomes a left-button press/release.
fn touch_as_mouse(
    touches: Res<Touches>,
    window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cursor_moved: MessageWriter<CursorMoved>,
    mut mouse_button: MessageWriter<MouseButtonInput>,
) {
    let (entity, mut window) = window.into_inner();
    if let Some(touch) = touches.iter().next() {
        window.set_cursor_position(Some(touch.position()));
        cursor_moved.write(CursorMoved {
            window: entity,
            position: touch.position(),
            delta: Some(touch.delta()),
        });
    }
    for touch in touches.iter_just_pressed() {
        window.set_cursor_position(Some(touch.position()));
        cursor_moved.write(CursorMoved {
            window: entity,
            position: touch.position(),
            delta: None,
        });
        mouse_button.write(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window: entity,
        });
    }
    for _ in touches.iter_just_released() {
        mouse_button.write(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window: entity,
        });
    }
}

fn respawn_when_dead(world: &mut World) {
    if session::is_dead(world)
        && world
            .resource::<ButtonInput<KeyCode>>()
            .get_just_pressed()
            .next()
            .is_some()
    {
        session::respawn(world);
    }
}
