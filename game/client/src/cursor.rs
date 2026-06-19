use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use world::area;
use world::math::{Pos, Tiles};
use world::protocol::AreaTag;
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

/// The cursor to show and the tile to highlight. A hovered UI widget (or an in-progress window
/// resize) takes over the cursor and clears the highlight; otherwise both follow the world.
fn desired(world: &mut World) -> (CursorIcon, Option<Pos<Tiles>>) {
    if let Some(ui) = bevy_view::hovered_cursor(world) {
        return (ui, None);
    }
    let (pointer, hover) = compute(world);
    (gameplay_cursor(world, pointer), hover)
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
    let walkable = session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()))
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

fn gameplay_cursor(world: &World, pointer: Pointer) -> CursorIcon {
    let cursors = world.resource::<Cursors>();
    // The default pointer's tip sits at the image's top-left; the others are centered 64×64 motifs.
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
            transform.translation.x = (tile.x + 0.5) * TILE.0;
            transform.translation.y = -(tile.y + 0.5) * TILE.0;
        }
        None => *visibility = Visibility::Hidden,
    }
}
