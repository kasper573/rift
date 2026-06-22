mod attack;
mod default;
mod drag;
mod walk;

pub use attack::AttackGesture;
pub use default::DefaultGesture;
pub use drag::DragGesture;
pub use walk::WalkGesture;

use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use enum_dispatch::enum_dispatch;

use crate::GameScene;

/// One way the primary button can be used. The input layer tries the active gestures in order and the
/// first to claim a press owns it until release, so two never run at once; a press no gesture claims is
/// left untouched. A new gesture is a new file here plus a [`GestureKind`] variant — this module never
/// learns a gesture's specifics.
#[enum_dispatch]
pub trait Gesture {
    /// Does this claim the press starting now? Reads the world — the cursor and what is under it.
    fn claims(&self, world: &mut World) -> bool;
    /// Drive the claimed press: `start` is true on the down-frame, false on each held frame after.
    fn drive(&mut self, world: &mut World, start: bool);
    /// The cursor to show while this would claim the press; `None` defers to the next gesture.
    fn cursor(&self, world: &mut World) -> Option<CursorIcon>;
}

/// The set of every gesture, most specific first. The one place that names them — both the active list
/// and the type discriminant systems match on (e.g. the crosshair). `enum_dispatch` forwards [`Gesture`]
/// to the active variant.
#[enum_dispatch(Gesture)]
pub enum GestureKind {
    Drag(DragGesture),
    Attack(AttackGesture),
    Walk(WalkGesture),
    Default(DefaultGesture),
}

impl GestureKind {
    /// Every gesture, constructed (each loads whatever assets it needs), most specific first.
    fn all(assets: &AssetServer) -> Vec<GestureKind> {
        vec![
            DragGesture.into(),
            AttackGesture::new(assets).into(),
            WalkGesture::new(assets).into(),
            DefaultGesture::new(assets).into(),
        ]
    }
}

/// Registers the whole gesture system; `input.rs` only wires this into the schedule.
pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, update.run_if(in_state(GameScene::Playing)));
}

/// The active gestures, built once at startup. The systems lift this out of the world to run
/// claims/drive/cursor against `&mut World`.
#[derive(Resource)]
pub(crate) struct Gestures(pub Vec<GestureKind>);

/// Which gesture in [`Gestures`] owns the press in progress.
#[derive(Resource, Default)]
struct Latched(Option<GestureIndex>);

#[derive(Clone, Copy)]
struct GestureIndex(usize);

/// The cursor currently applied to the window, so the same one isn't re-inserted every frame.
#[derive(Resource, Default)]
struct AppliedCursor(Option<CursorIcon>);

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Gestures(GestureKind::all(&assets)));
    commands.init_resource::<Latched>();
    commands.init_resource::<AppliedCursor>();
}

/// The whole gesture step in one pass: latch the first gesture to claim the press and drive it until
/// release, then show the first claiming gesture's cursor (`DefaultGesture` always supplies one).
fn update(world: &mut World) {
    let mut gestures = world.remove_resource::<Gestures>().expect("gestures");

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
            .0
            .iter()
            .position(|gesture| gesture.claims(world))
            .map(GestureIndex);
    }
    if let Some(GestureIndex(index)) = active {
        gestures.0[index].drive(world, just);
    }
    world.resource_mut::<Latched>().0 = active;

    let mut cursor = None;
    for gesture in &gestures.0 {
        if gesture.claims(world) {
            cursor = gesture.cursor(world);
            if cursor.is_some() {
                break;
            }
        }
    }

    world.insert_resource(gestures);
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
