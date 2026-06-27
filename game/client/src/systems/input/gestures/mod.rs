mod attack;
mod default;
mod drag;
mod pickup;
mod walk;

use super::ActiveTileHighlight;
use crate::Scene;
use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};

pub trait Gesture: Send + Sync {
    fn priority(&self) -> i32;
    fn claims(&self, world: &mut World) -> bool;
    fn drive(&self, world: &mut World, start: bool);
    fn cursor(&self, world: &mut World) -> Option<CursorIcon>;
    fn tile_highlight(&self, _world: &mut World) -> Option<ActiveTileHighlight> {
        None
    }
}

static GESTURES: &[&dyn Gesture] = &[
    &attack::AttackGesture,
    &default::DefaultGesture,
    &drag::DragGesture,
    &pickup::PickupGesture,
    &walk::WalkGesture,
];

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, update.run_if(in_state(Scene::Area)));
}

#[derive(Resource)]
pub(crate) struct Gestures(pub Vec<&'static dyn Gesture>);

#[derive(Resource, Default)]
struct Latched(Option<GestureIndex>);

#[derive(Clone, Copy)]
struct GestureIndex(usize);

#[derive(Resource, Default)]
struct AppliedCursor(Option<CursorIcon>);

fn setup(mut commands: Commands) {
    let mut gestures: Vec<&'static dyn Gesture> = GESTURES.to_vec();
    gestures.sort_by_key(|gesture| gesture.priority());
    commands.insert_resource(Gestures(gestures));
    commands.init_resource::<Latched>();
    commands.init_resource::<AppliedCursor>();
}

fn update(world: &mut World) {
    let gestures = world.resource::<Gestures>().0.clone();

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
        active = gestures
            .iter()
            .position(|gesture| gesture.claims(world))
            .map(GestureIndex);
    }
    if let Some(GestureIndex(index)) = active {
        gestures[index].drive(world, just);
    }
    world.resource_mut::<Latched>().0 = active;

    let mut cursor = None;
    let mut highlight = None;
    let mut first_claimant = true;
    for gesture in &gestures {
        if !gesture.claims(world) {
            continue;
        }
        if first_claimant {
            highlight = gesture.tile_highlight(world);
            first_claimant = false;
        }
        cursor = gesture.cursor(world);
        if cursor.is_some() {
            break;
        }
    }

    match highlight {
        Some(highlight) => world.insert_resource(highlight),
        None => {
            world.remove_resource::<ActiveTileHighlight>();
        }
    }
    apply_cursor(world, cursor);
}

fn apply_cursor(world: &mut World, cursor: Option<CursorIcon>) {
    let Some(cursor) = cursor else {
        return;
    };
    if world.resource::<AppliedCursor>().0.as_ref() == Some(&cursor) {
        return;
    }
    world.resource_mut::<AppliedCursor>().0 = Some(cursor.clone());
    if let Ok(window) = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
    {
        world.entity_mut(window).insert(cursor);
    }
}

pub(crate) fn image_cursor(handle: Handle<Image>, hotspot: (u16, u16)) -> CursorIcon {
    CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle,
        texture_atlas: None,
        flip_x: false,
        flip_y: false,
        rect: None,
        hotspot,
    }))
}
