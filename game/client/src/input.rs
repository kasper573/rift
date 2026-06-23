pub mod gestures;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use world::protocol::session;

use crate::GameScene;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        gestures::plugin(app);
        app.add_systems(Update, touch_as_mouse);
        app.add_systems(
            Update,
            respawn_when_dead.run_if(in_state(GameScene::Playing)),
        );
    }
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
