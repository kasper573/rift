use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use world::area;
use world::math::{Pos, Tiles};
use world::session;

use crate::Screen;
use crate::render::TILE;
use crate::view;

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, update.run_if(in_state(Screen::Playing)));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pointer {
    Default,
    Attack,
    Move,
    MoveHeld,
}

#[derive(Resource)]
struct Cursors {
    default: Handle<Image>,
    attack: Handle<Image>,
    walk: Handle<Image>,
    walk_held: Handle<Image>,
    current: Option<Pointer>,
}

#[derive(Component)]
struct Hover;

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Cursors {
        default: assets.load("icons/cursors/pointer003.png"),
        attack: assets.load("icons/cursors/swords002.png"),
        walk: assets.load("icons/cursors/pointer010.png"),
        walk_held: assets.load("icons/cursors/pointer011.png"),
        current: None,
    });
    commands.spawn((
        Hover,
        Sprite {
            image: assets.load("icons/crosshairs/white/crosshair026.png"),
            custom_size: Some(Vec2::splat(TILE)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 100.0),
        Visibility::Hidden,
    ));
}

fn update(world: &mut World) {
    let (pointer, hover) = compute(world);
    apply_cursor(world, pointer);
    apply_hover(world, hover);
}

/// The pointer the cursor should show and the walkable tile to highlight (if any).
fn compute(world: &mut World) -> (Pointer, Option<Pos<Tiles>>) {
    if session::is_dead(world) {
        return (Pointer::Default, None);
    }
    let Some(point) = view::cursor_tile(world) else {
        return (Pointer::Default, None);
    };
    if view::enemy_at(world, point).is_some() {
        return (Pointer::Attack, None);
    }
    let tile = Pos::new(point.x.floor(), point.y.floor());
    let walkable = session::my_area(world)
        .and_then(|id| area::areas().get(id.0 as usize))
        .is_some_and(|area| area.grid.walkable(tile));
    if !walkable {
        return (Pointer::Default, None);
    }
    let held = world
        .resource::<ButtonInput<MouseButton>>()
        .pressed(MouseButton::Left);
    let pointer = if held {
        Pointer::MoveHeld
    } else {
        Pointer::Move
    };
    (pointer, Some(tile))
}

fn apply_cursor(world: &mut World, pointer: Pointer) {
    if world.resource::<Cursors>().current == Some(pointer) {
        return;
    }
    let cursors = world.resource::<Cursors>();
    // The default pointer's tip sits at the image's top-left; the others are centered 64×64 motifs.
    let (handle, hotspot) = match pointer {
        Pointer::Default => (cursors.default.clone(), (0, 0)),
        Pointer::Attack => (cursors.attack.clone(), (32, 32)),
        Pointer::Move => (cursors.walk.clone(), (32, 32)),
        Pointer::MoveHeld => (cursors.walk_held.clone(), (32, 32)),
    };
    world.resource_mut::<Cursors>().current = Some(pointer);
    let icon = CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle,
        texture_atlas: None,
        flip_x: false,
        flip_y: false,
        rect: None,
        hotspot,
    }));
    if let Ok(window) = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
    {
        world.entity_mut(window).insert(icon);
    }
}

fn apply_hover(world: &mut World, hover: Option<Pos<Tiles>>) {
    let mut hovers = world.query_filtered::<(&mut Transform, &mut Visibility), With<Hover>>();
    let Ok((mut transform, mut visibility)) = hovers.single_mut(world) else {
        return;
    };
    match hover {
        Some(tile) => {
            *visibility = Visibility::Visible;
            transform.translation.x = (tile.x + 0.5) * TILE;
            transform.translation.y = -(tile.y + 0.5) * TILE;
        }
        None => *visibility = Visibility::Hidden,
    }
}
