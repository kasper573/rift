use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use world::math::Pos;
use world::session;
use world::tiling::{TilePos, Tiles};

use crate::Screen;
use crate::render::{self, TILE};
use crate::screen::ToScreen;
use world::query;

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
    applied: Option<CursorIcon>,
}

#[derive(Component)]
struct Hover;

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Cursors {
        default: assets.load("icons/cursors/pointer003.png"),
        attack: assets.load("icons/cursors/swords002.png"),
        walk: assets.load("icons/cursors/pointer010.png"),
        walk_held: assets.load("icons/cursors/pointer011.png"),
        applied: None,
    });
    commands.spawn((
        Hover,
        Sprite {
            image: assets.load("icons/crosshairs/white/crosshair026.png"),
            custom_size: Some(Vec2::splat(TILE.0)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 100.0),
        Visibility::Hidden,
    ));
}

fn update(world: &mut World) {
    let (icon, hover) = desired(world);
    set_cursor(world, icon);
    apply_hover(world, hover);
}

fn desired(world: &mut World) -> (CursorIcon, Option<Pos<Tiles>>) {
    if let Some(ui) = ui::hovered_cursor(world) {
        return (ui, None);
    }
    let (pointer, hover) = compute(world);
    (gameplay_cursor(world, pointer), hover)
}

fn compute(world: &mut World) -> (Pointer, Option<Pos<Tiles>>) {
    if session::is_dead(world) {
        return (Pointer::Default, None);
    }
    let Some(point) = render::cursor_tile(world) else {
        return (Pointer::Default, None);
    };
    if query::enemy_at(world, point).is_some() {
        return (Pointer::Attack, None);
    }
    let tile = point.snap();
    if !query::walkable(world, tile) {
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

fn gameplay_cursor(world: &World, pointer: Pointer) -> CursorIcon {
    let cursors = world.resource::<Cursors>();
    let (handle, hotspot) = match pointer {
        Pointer::Default => (cursors.default.clone(), (0, 0)),
        Pointer::Attack => (cursors.attack.clone(), (32, 32)),
        Pointer::Move => (cursors.walk.clone(), (32, 32)),
        Pointer::MoveHeld => (cursors.walk_held.clone(), (32, 32)),
    };
    CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle,
        texture_atlas: None,
        flip_x: false,
        flip_y: false,
        rect: None,
        hotspot,
    }))
}

fn set_cursor(world: &mut World, icon: CursorIcon) {
    if world.resource::<Cursors>().applied.as_ref() == Some(&icon) {
        return;
    }
    world.resource_mut::<Cursors>().applied = Some(icon.clone());
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
            let p = tile.to_screen();
            transform.translation.x = p.x;
            transform.translation.y = p.y;
        }
        None => *visibility = Visibility::Hidden,
    }
}
