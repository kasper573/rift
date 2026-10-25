pub mod camera;
mod escape;
pub mod gestures;
pub mod map;
pub mod marks;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use bevy_egui::input::egui_wants_any_keyboard_input;

use crate::systems::player::RespawnRequest;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

pub struct PlayerInputPlugin;

impl Plugin for PlayerInputPlugin {
    fn build(&self, app: &mut App) {
        map::plugin(app);
        gestures::plugin(app);
        marks::plugin(app);
        camera::plugin(app);
        escape::plugin(app);
        app.add_systems(Update, touch_as_mouse).add_systems(
            Update,
            respawn_when_dead
                .run_if(not(egui_wants_any_keyboard_input))
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

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
    if session::is_dead(world) && map::just_pressed(world, map::InputAction::Respawn) {
        world.write_message(RespawnRequest);
    }
}
