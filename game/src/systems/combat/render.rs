use crate::core::math::{Size, WorldPx};
use crate::systems::player::session;
use crate::systems::stat;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::core::render::present::ScreenTint;
use crate::core::render::screen::ToScreen;
use crate::core::render::snap_to_screen;
use crate::systems::interpolate::RenderPosition;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_bar).add_systems(
            Update,
            (healthbar, death_tint).run_if(in_state(crate::systems::scene::Scene::Area)),
        );
    }
}

#[derive(Component, Clone, Copy)]
enum Bar {
    Border,
    Background,
    Fill,
}

const BAR: Size<WorldPx> = Size::new(20.0, 4.0);
const BAR_DROP: WorldPx = WorldPx(5.0);

fn spawn_bar(mut commands: Commands) {
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
    let shown = session::me(world)
        .and_then(|me| Some((me.id(), me.get::<RenderPosition>()?.0)))
        .and_then(|(entity, at)| {
            (!stat::is_dead(world, entity) && stat::max_health(world, entity) > 0.0).then(|| {
                (
                    snap_to_screen(at.to_screen() - Vec2::new(0.0, BAR_DROP.0)),
                    stat::fraction(world, entity),
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

fn death_tint(world: &mut World) {
    let tint = if session::is_dead(world) {
        Vec4::new(1.0, 0.0, 0.0, 0.0)
    } else {
        Vec4::ZERO
    };
    world.resource_mut::<ScreenTint>().0 = tint;
}
