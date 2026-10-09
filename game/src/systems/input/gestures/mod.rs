mod attack;
mod default;
mod interact;
mod interface;
mod locked;
mod pickup;
mod walk;

use super::ActiveTileHighlight;
use super::map::{self, InputAction};
use crate::core::render::transition::WorldViewSystems;
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
    &interact::InteractGesture,
    &interface::InterfaceGesture,
    &locked::LockedGesture,
    &pickup::PickupGesture,
    &walk::WalkGesture,
];

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, update.in_set(WorldViewSystems));
}

#[derive(Resource)]
pub(crate) struct Gestures(pub Vec<&'static dyn Gesture>);

#[derive(Resource, Default)]
struct Latched(Option<GestureIndex>);

#[derive(Resource, Default)]
struct LockSeen(bool);

#[derive(Clone, Copy)]
struct GestureIndex(usize);

#[derive(Resource, Default)]
struct AppliedCursor(Option<CursorIcon>);

fn setup(mut commands: Commands) {
    let mut gestures: Vec<&'static dyn Gesture> = GESTURES.to_vec();
    gestures.sort_by_key(|gesture| gesture.priority());
    commands.insert_resource(Gestures(gestures));
    commands.init_resource::<Latched>();
    commands.init_resource::<LockSeen>();
    commands.init_resource::<AppliedCursor>();
}

fn update(world: &mut World) {
    let gestures = world.resource::<Gestures>().0.clone();
    forget_clicks_across_lock(world);

    let mut active = world.resource::<Latched>().0;
    let pressed = map::pressed(world, InputAction::Interact);
    let just = map::just_pressed(world, InputAction::Interact);
    // A click pressed and released between two frames arrives as `just` without `pressed`.
    if just {
        active = gestures
            .iter()
            .position(|gesture| gesture.claims(world))
            .map(GestureIndex);
    } else if !pressed {
        active = None;
    }
    if let Some(GestureIndex(index)) = active {
        gestures[index].drive(world, just);
    }
    world.resource_mut::<Latched>().0 = active.filter(|_| pressed);

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

fn forget_clicks_across_lock(world: &mut World) {
    let locked = crate::systems::player::session::is_locked(world);
    if std::mem::replace(&mut world.resource_mut::<LockSeen>().0, locked) != locked {
        world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
        world.resource_mut::<Latched>().0 = None;
    }
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

pub(crate) fn default_cursor(world: &mut World) -> CursorIcon {
    let handle = world
        .resource::<AssetServer>()
        .load("icons/cursors/pointer003.png");
    image_cursor(handle, (0, 0))
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
