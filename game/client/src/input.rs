use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use world::session;

use crate::Screen;
use crate::view;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, click_to_act.run_if(in_state(Screen::Playing)));
        app.add_systems(Update, respawn_when_dead.run_if(in_state(Screen::Playing)));
    }
}

fn click_to_act(world: &mut World) {
    if !world
        .resource::<ButtonInput<MouseButton>>()
        .just_pressed(MouseButton::Left)
        || session::is_dead(world)
        || pointer_on_ui(world)
    {
        return;
    }
    let Some(point) = view::cursor_tile(world) else {
        return;
    };
    match view::enemy_at(world, point) {
        Some(target) => session::attack(world, target),
        None => session::move_to(world, point),
    }
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
