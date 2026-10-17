mod attack;
mod default;
mod interact;
mod interface;
mod locked;
mod pickup;
mod walk;

use bevy::prelude::*;
use bevy::window::{
    CursorIcon, CursorOptions, CustomCursor, CustomCursorImage, PrimaryWindow, SystemCursorIcon,
};
use bevy_egui::EguiPrimaryContextPass;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use super::ActiveTileHighlight;
use super::map::{self, InputAction};
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::render;
use crate::core::render::transition::WorldViewSystems;
use crate::core::tiling::Tiles;
use crate::systems::interface::CursorShape;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum ClickClaim {
    Interface,
    Locked,
    Attack(Entity),
    Aim(Entity),
    AimAt(Pos<Tiles>),
    Pickup(Entity),
    Interact(Entity),
    Walk(Pos<Tiles>),
    Nothing,
}

pub fn claim(world: &mut World) -> ClickClaim {
    let tile = render::cursor_tile(world);
    claim_at(world, tile)
}

pub fn claim_at(world: &mut World, tile: Option<Pos<Tiles>>) -> ClickClaim {
    GESTURES
        .iter()
        .find_map(|gesture| gesture.claim(world, tile))
        .unwrap_or(ClickClaim::Nothing)
}

/// How the pointer looks: one of the game's own cursors, or the system's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cursor {
    Shaped(CursorShape),
    System(SystemCursorIcon),
}

/// A gesture takes a click by claiming it from the tile under the pointer, which is `None` while the
/// pointer is off the world or over the interface.
pub trait Gesture: Send + Sync {
    fn claim(&self, world: &mut World, tile: Option<Pos<Tiles>>) -> Option<ClickClaim>;
    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool);
    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor>;
    fn tile_highlight(
        &self,
        _world: &mut World,
        _claim: ClickClaim,
    ) -> Option<ActiveTileHighlight> {
        None
    }
}

// The first gesture to claim a click takes it.
static GESTURES: &[&dyn Gesture] = &[
    &interface::InterfaceGesture,
    &locked::LockedGesture,
    &crate::systems::skill::aim::AimGesture,
    &attack::AttackGesture,
    &pickup::PickupGesture,
    &interact::InteractGesture,
    &walk::WalkGesture,
    &default::DefaultGesture,
];

pub fn plugin(app: &mut App) {
    app.init_resource::<Latched>()
        .init_resource::<LockSeen>()
        .init_resource::<ChosenCursor>()
        .init_resource::<ShownCursor>()
        .add_systems(
            Update,
            load_cursors.run_if(resource_exists_and_changed::<Content>),
        )
        .add_systems(
            Update,
            (
                update
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
                show_cursor.after(update),
            ),
        )
        .add_systems(EguiPrimaryContextPass, draw_cursor.in_set(Band::Cursor));
}

#[derive(Resource, Default)]
struct Latched(Option<GestureIndex>);

#[derive(Resource, Default)]
struct LockSeen(bool);

#[derive(Clone, Copy)]
struct GestureIndex(usize);

#[derive(Resource, Default)]
/// The cursor the area's gestures chose for this frame, `None` where no gesture runs.
struct ChosenCursor(Option<Cursor>);

/// The system draws its cursor as the pointer moves, but a frame reaches the screen a few frames
/// after it reads the pointer, so whatever the interface drags would trail a system cursor. While
/// something is dragged, the cursor is drawn in the frame instead, with what it drags.
#[derive(Resource, Default, PartialEq, Clone)]
struct ShownCursor {
    icon: Option<CursorIcon>,
    in_frame: Option<CursorShape>,
}

// Held for the whole session: a cursor loaded by path on each switch is unloaded as soon as the
// shape changes, and the next switch back finds it missing again.
#[derive(Resource)]
struct CursorImages(Vec<(CursorShape, Handle<Image>)>);

fn load_cursors(content: Res<Content>, assets: Res<AssetServer>, mut commands: Commands) {
    let images = CursorShape::VARIANTS
        .iter()
        .map(|&shape| (shape, assets.load(shape.get(&content).image.path())))
        .collect();
    commands.insert_resource(CursorImages(images));
}

fn update(world: &mut World) {
    forget_clicks_across_lock(world);

    let tile = render::cursor_tile(world);
    let claims: Vec<Option<ClickClaim>> = GESTURES
        .iter()
        .map(|gesture| gesture.claim(world, tile))
        .collect();
    let mut active = world.resource::<Latched>().0;
    let pressed = map::pressed(world, InputAction::Interact);
    let just = map::just_pressed(world, InputAction::Interact);
    // A click pressed and released between two frames arrives as `just` without `pressed`.
    if just {
        active = claims.iter().position(Option::is_some).map(GestureIndex);
    } else if !pressed {
        active = None;
    }
    if let Some(GestureIndex(index)) = active {
        GESTURES[index].drive(world, claims[index], just);
    }
    world.resource_mut::<Latched>().0 = active.filter(|_| pressed);

    let mut cursor = None;
    let mut highlight = None;
    let mut first_claimant = true;
    for (gesture, claim) in GESTURES.iter().zip(&claims) {
        let Some(claim) = *claim else {
            continue;
        };
        if first_claimant {
            highlight = gesture.tile_highlight(world, claim);
            first_claimant = false;
        }
        cursor = gesture.cursor(world, claim);
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
    world.resource_mut::<ChosenCursor>().0 = cursor;
}

fn forget_clicks_across_lock(world: &mut World) {
    let locked = session::is_locked(world);
    if std::mem::replace(&mut world.resource_mut::<LockSeen>().0, locked) != locked {
        world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
        world.resource_mut::<Latched>().0 = None;
    }
}

fn show_cursor(world: &mut World) {
    let in_area = *world.resource::<State<ClientScreen>>().get() == ClientScreen::Area;
    let chosen = in_area
        .then(|| world.resource::<ChosenCursor>().0)
        .flatten();
    let shown = match chosen {
        None => ShownCursor {
            icon: Some(CursorIcon::System(SystemCursorIcon::Default)),
            in_frame: None,
        },
        Some(Cursor::Shaped(shape)) if gui::dragging(world) => ShownCursor {
            icon: None,
            in_frame: Some(shape),
        },
        Some(Cursor::Shaped(shape)) => {
            let icon = content_cursor(world, shape);
            if let CursorIcon::Custom(CustomCursor::Image(image)) = &icon
                && !world.resource::<Assets<Image>>().contains(&image.handle)
            {
                return;
            }
            ShownCursor {
                icon: Some(icon),
                in_frame: None,
            }
        }
        Some(Cursor::System(icon)) => ShownCursor {
            icon: Some(CursorIcon::System(icon)),
            in_frame: None,
        },
    };
    if *world.resource::<ShownCursor>() == shown {
        return;
    }
    world.insert_resource(shown.clone());
    let Ok(window) = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
    else {
        return;
    };
    let mut window = world.entity_mut(window);
    if let Some(icon) = shown.icon {
        window.insert(icon);
    }
    if let Some(mut options) = window.get_mut::<CursorOptions>() {
        options.visible = shown.in_frame.is_none();
    }
}

fn draw_cursor(world: &mut World) {
    let Some(shape) = world.resource::<ShownCursor>().in_frame else {
        return;
    };
    let Some(gui) = gui::context(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let cursor = shape.get(&content);
    let hotspot = egui::vec2(f32::from(cursor.hotspot.x), f32::from(cursor.hotspot.y));
    let Some(image) = gui::image(world, &cursor.image) else {
        return;
    };
    gui::at_pointer(
        &gui,
        egui::Id::new("drawn cursor"),
        Band::Cursor,
        -hotspot,
        |ui| ui.add(image.widget(image.size)),
    );
}

fn content_cursor(world: &World, shape: CursorShape) -> CursorIcon {
    let cursor = shape.get(world.resource::<Content>());
    let handle = world
        .get_resource::<CursorImages>()
        .and_then(|images| images.0.iter().find(|(known, _)| *known == shape))
        .map(|(_, handle)| handle.clone())
        .unwrap_or_default();
    CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle,
        texture_atlas: None,
        flip_x: false,
        flip_y: false,
        rect: None,
        hotspot: (cursor.hotspot.x, cursor.hotspot.y),
    }))
}
