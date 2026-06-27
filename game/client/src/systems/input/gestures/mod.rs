mod attack;
mod default;
mod drag;
mod pickup;
mod walk;

use super::ActiveTileHighlight;
use crate::Scene;
use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};

/// One way the primary button can be used. The input layer tries the active gestures in order and the
/// first to claim a press owns it until release, so two never run at once; a press no gesture claims is
/// left untouched.
pub trait Gesture: Send + Sync {
    /// Where this sits in the claim order — lower is tried first, so the most specific gesture wins.
    fn priority(&self) -> i32;
    /// Does this claim the press starting now? Reads the world — the cursor and what is under it.
    fn claims(&self, world: &mut World) -> bool;
    /// Drive the claimed press: `start` is true on the down-frame, false on each held frame after.
    fn drive(&self, world: &mut World, start: bool);
    /// The cursor to show while this would claim the press; `None` defers to the next gesture.
    fn cursor(&self, world: &mut World) -> Option<CursorIcon>;
    /// The tile pos and image this gesture marks while it is the active gesture.
    fn tile_highlight(&self, _world: &mut World) -> Option<ActiveTileHighlight> {
        None
    }
}

inventory::collect!(&'static dyn Gesture);

/// Registers the whole gesture system; `input.rs` only wires this into the schedule.
pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, update.run_if(in_state(Scene::Area)));
}

/// Every gesture, ordered most-specific-first (by [`Gesture::priority`]), built once at startup.
#[derive(Resource)]
pub(crate) struct Gestures(pub Vec<&'static dyn Gesture>);

/// Which gesture in [`Gestures`] owns the press in progress.
#[derive(Resource, Default)]
struct Latched(Option<GestureIndex>);

#[derive(Clone, Copy)]
struct GestureIndex(usize);

/// The cursor currently applied to the window, so the same one isn't re-inserted every frame.
#[derive(Resource, Default)]
struct AppliedCursor(Option<CursorIcon>);

fn setup(mut commands: Commands) {
    let mut gestures: Vec<&'static dyn Gesture> =
        inventory::iter::<&'static dyn Gesture>().copied().collect();
    gestures.sort_by_key(|gesture| gesture.priority());
    commands.insert_resource(Gestures(gestures));
    commands.init_resource::<Latched>();
    commands.init_resource::<AppliedCursor>();
}

/// The whole gesture step in one pass: latch the first gesture to claim the press and drive it until
/// release, then show the first claiming gesture's cursor (`DefaultGesture` always supplies one).
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

    // One descending pass over the claimants: the first (the active gesture) owns the crosshair; the
    // cursor falls through to the first claimant that supplies one (`DefaultGesture` always does).
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

/// A custom image cursor with the given hotspot — the shared shape the gameplay gestures fill.
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
