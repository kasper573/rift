//! The death banner: a full-screen prompt shown while the local player is dead, cleared on respawn.

use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::text_colored;
use world::player::session;

use super::Hud;

#[derive(Component, Default, Clone)]
struct DeathBanner;

pub(super) fn sync_death_banner(world: &mut World) {
    let dead = session::is_dead(world);
    let banner = world
        .query_filtered::<Entity, With<DeathBanner>>()
        .iter(world)
        .next();
    match (dead, banner) {
        (true, None) => {
            if let Some(hud) = world
                .query_filtered::<Entity, With<Hud>>()
                .iter(world)
                .next()
                && let Ok(spawned) = world.spawn_scene(death_banner())
            {
                let banner = spawned.id();
                world.entity_mut(hud).add_child(banner);
            }
        }
        (false, Some(banner)) => world.entity_mut(banner).despawn(),
        _ => {}
    }
}

fn death_banner() -> impl Scene {
    bsn! {
        DeathBanner
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        GlobalZIndex({50})
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {EntityScene(text_colored("You died! Press any key to respawn", Color::WHITE))} ]
    }
}
