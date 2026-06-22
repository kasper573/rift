use std::time::Duration;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use world::math::Pos;
use world::session;
use world::tiling::{TilePos, Tiles};

use crate::Screen;
use crate::view;

const MOVE_REPEAT: Duration = Duration::from_millis(333);

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HeldMove>();
        app.add_systems(Update, touch_as_mouse);
        app.add_systems(Update, click_to_act.run_if(in_state(Screen::Playing)));
        app.add_systems(Update, respawn_when_dead.run_if(in_state(Screen::Playing)));
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

#[derive(Resource, Default)]
struct HeldMove {
    last_tile: Option<Pos<Tiles>>,
    last_sent: Option<Duration>,
}

fn click_to_act(world: &mut World) {
    if session::is_dead(world) || pointer_on_ui(world) {
        return;
    }
    let buttons = world.resource::<ButtonInput<MouseButton>>();
    if !buttons.pressed(MouseButton::Left) {
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        press(world);
    } else {
        repeat_move(world);
    }
}

fn press(world: &mut World) {
    let Some(point) = view::cursor_tile(world) else {
        return;
    };
    match view::enemy_at(world, point) {
        Some(target) => {
            session::attack(world, target);
            stamp(world, None);
        }
        None => {
            session::move_to(world, point);
            stamp(world, Some(point.snap()));
        }
    }
}

fn repeat_move(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    if world
        .resource::<HeldMove>()
        .last_sent
        .is_some_and(|sent| now.saturating_sub(sent) < MOVE_REPEAT)
    {
        return;
    }
    let Some(point) = view::cursor_tile(world) else {
        return;
    };
    if view::enemy_at(world, point).is_some() {
        return;
    }
    let tile = point.snap();
    if !view::walkable(world, tile) || world.resource::<HeldMove>().last_tile == Some(tile) {
        return;
    }
    session::move_to(world, point);
    stamp(world, Some(tile));
}

fn stamp(world: &mut World, tile: Option<Pos<Tiles>>) {
    let now = world.resource::<Time>().elapsed();
    let mut state = world.resource_mut::<HeldMove>();
    state.last_tile = tile;
    state.last_sent = Some(now);
}

fn pointer_on_ui(world: &World) -> bool {
    world
        .resource::<HoverMap>()
        .values()
        .flat_map(|hits| hits.keys())
        .any(|&entity| world.get::<Node>(entity).is_some())
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
