//! Bespoke world- and screen-space effects: the local player's floating health bar, the active
//! gesture's tile highlight, and the death screen tint. All specific to this game, drawn on top of the
//! generic render pipeline in [`crate::core::render`].

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::math::{Pos, Size, WorldPx};
use world::core::tiling::Tiles;
use world::systems::combat::Vitals;
use world::systems::movement::Position;
use world::systems::player::session;

use crate::core::render::TILE;
use crate::core::render::present::ScreenTint;
use crate::core::render::screen::ToScreen;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (healthbar, update_tile_highlight, death_tint)
                .run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

/// What the active gesture wants highlighted on the map — a tile and the image to mark it with. The
/// gesture system (`crate::systems::input`) sets it; [`update_tile_highlight`] draws it.
#[derive(Resource)]
pub struct ActiveTileHighlight {
    pub pos: Pos<Tiles>,
    pub image: Handle<Image>,
}

#[derive(Component, Clone, Copy)]
enum Bar {
    Border,
    Background,
    Fill,
}

const BAR: Size<WorldPx> = Size::new(20.0, 4.0);
const BAR_DROP: WorldPx = WorldPx(5.0);

#[derive(Component)]
struct TileHighlight;

fn setup(mut commands: Commands) {
    commands.spawn((
        Bar::Border,
        bar_sprite(0x14_0A_28, BAR),
        Anchor::CENTER,
        hidden(),
    ));
    let inner = BAR - Size::splat(2.0);
    commands.spawn((
        Bar::Background,
        bar_sprite(0x2A_1C_5C, inner),
        Anchor::CENTER,
        hidden(),
    ));
    commands.spawn((
        Bar::Fill,
        bar_sprite(0x00_FF_00, inner),
        Anchor::CENTER_LEFT,
        hidden(),
    ));
    commands.spawn((
        TileHighlight,
        Sprite {
            custom_size: Some(Vec2::splat(TILE.0)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 100.0),
        Visibility::Hidden,
    ));
}

fn bar_sprite(rgb: u32, size: Size<WorldPx>) -> Sprite {
    let [_, r, g, b] = rgb.to_be_bytes();
    Sprite {
        color: Color::srgb_u8(r, g, b),
        custom_size: Some(Vec2::new(size.width, size.height)),
        ..default()
    }
}

fn hidden() -> (Transform, Visibility) {
    (Transform::from_xyz(0.0, 0.0, 200.0), Visibility::Hidden)
}

fn healthbar(world: &mut World) {
    let shown = session::me(world).and_then(|me| {
        // Whole pixels: avoid floor/ceil alternation on fractional offsets during movement.
        let at = me.get::<Position>()?.pos;
        let vitals = me.get::<Vitals>()?;
        (!vitals.is_dead() && vitals.max > 0.0).then(|| {
            (
                (at.to_screen() - Vec2::new(0.0, BAR_DROP.0)).round(),
                vitals.fraction(),
            )
        })
    });
    let mut bars = world.query::<(&Bar, &mut Transform, &mut Visibility, &mut Sprite)>();
    for (bar, mut transform, mut visibility, mut sprite) in bars.iter_mut(world) {
        let Some((center, fraction)) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;
        match bar {
            Bar::Border => transform.translation = center.extend(200.0),
            Bar::Background => transform.translation = center.extend(200.1),
            Bar::Fill => {
                let inner = BAR.width - 2.0;
                sprite.custom_size = Some(Vec2::new((inner * fraction).floor(), BAR.height - 2.0));
                transform.translation = Vec3::new(center.x - inner / 2.0, center.y, 200.2);
            }
        }
    }
}

/// Renders the tile highlight the active gesture published — appearance and position both come from the
/// gesture; this just mirrors them onto the sprite.
fn update_tile_highlight(
    highlight: Option<Res<ActiveTileHighlight>>,
    mut sprite: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<TileHighlight>>,
) {
    let Ok((mut sprite, mut transform, mut visibility)) = sprite.single_mut() else {
        return;
    };
    match highlight {
        Some(highlight) => {
            *visibility = Visibility::Visible;
            sprite.image = highlight.image.clone();
            let at = highlight.pos.to_screen();
            transform.translation.x = at.x;
            transform.translation.y = at.y;
        }
        None => *visibility = Visibility::Hidden,
    }
}

/// Washes the whole screen on the local player's death by driving the generic [`ScreenTint`].
fn death_tint(world: &mut World) {
    let tint = if session::is_dead(world) {
        Vec4::new(1.0, 0.0, 0.0, 0.0)
    } else {
        Vec4::ZERO
    };
    world.resource_mut::<ScreenTint>().0 = tint;
}
