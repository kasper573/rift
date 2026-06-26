//! Item presentation: plays an item's use sound when the server confirms a use, renders the
//! [`DroppedItem`] entities loot leaves on the map, and fountains a fresh drop out from its origin —
//! each item arcing up and out to its tile, thudding with the item's drop sound on landing.

use bevy::prelude::*;
use world::core::math::Pos;
use world::core::tiling::Tiles;
use world::systems::area::{self, AreaTag};
use world::systems::item::{DroppedItem, ItemConsumed, ItemsDropped};
use world::systems::movement::Position;

use crate::core::audio::PlaySfx;
use crate::core::render::{ToScreen, dynamic_z, sprite_transform};

const DROP_SIZE: f32 = 12.0;
const DROP_STAGGER: f32 = 0.06;
const DROP_DURATION: f32 = 0.5;
const DROP_HOP: f32 = 24.0;
const PENDING_TTL: f32 = 0.5;

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingDrops>()
            .add_observer(attach_drop_sprite)
            .add_systems(
                Update,
                (
                    use_sounds,
                    (recv_drops, start_drops, place_drops, animate_drops).chain(),
                )
                    .run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

/// The in-flight fountain arc of one freshly dropped item, in screen space.
#[derive(Component)]
struct DropAnim {
    from: Vec2,
    to: Vec2,
    delay: f32,
    elapsed: f32,
    drop_sfx: Option<String>,
}

/// Drops named by an [`ItemsDropped`] whose entities haven't replicated in yet; retried for a few
/// frames so the fountain still plays through any replication/message ordering lag.
#[derive(Resource, Default)]
struct PendingDrops(Vec<Pending>);

struct Pending {
    entity: Entity,
    from: Pos<Tiles>,
    index: usize,
    age: f32,
}

fn use_sounds(
    mut consumed: MessageReader<ItemConsumed>,
    positions: Query<&Position>,
    mut play: MessageWriter<PlaySfx>,
) {
    for consumed in consumed.read() {
        let Some(id) = consumed.item.get().sfx.on_use.as_ref() else {
            continue;
        };
        let Ok(position) = positions.get(consumed.actor) else {
            continue;
        };
        play.write(PlaySfx {
            id: id.0.clone(),
            at: position.pos,
        });
    }
}

fn attach_drop_sprite(
    add: On<Add, DroppedItem>,
    drops: Query<&DroppedItem>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(dropped) = drops.get(add.entity) else {
        return;
    };
    let image = assets.load(dropped.item.get().icon.0.clone());
    commands.entity(add.entity).insert((
        Sprite {
            image,
            custom_size: Some(Vec2::splat(DROP_SIZE)),
            ..default()
        },
        Transform::default(),
        Visibility::default(),
    ));
}

fn recv_drops(mut dropped: MessageReader<ItemsDropped>, mut pending: ResMut<PendingDrops>) {
    for dropped in dropped.read() {
        for (index, &entity) in dropped.items.iter().enumerate() {
            pending.0.push(Pending {
                entity,
                from: dropped.from,
                index,
                age: 0.0,
            });
        }
    }
}

fn start_drops(
    time: Res<Time>,
    mut pending: ResMut<PendingDrops>,
    drops: Query<(&DroppedItem, &Position)>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    pending.0.retain_mut(|drop| match drops.get(drop.entity) {
        Ok((dropped, position)) => {
            commands.entity(drop.entity).insert(DropAnim {
                from: drop.from.to_screen(),
                to: position.pos.to_screen(),
                delay: drop.index as f32 * DROP_STAGGER,
                elapsed: 0.0,
                drop_sfx: dropped
                    .item
                    .get()
                    .sfx
                    .drop
                    .as_ref()
                    .map(|sfx| sfx.0.clone()),
            });
            false
        }
        Err(_) => {
            drop.age += dt;
            drop.age < PENDING_TTL
        }
    });
}

/// Rests every dropped item at its tile. An item mid-fountain is then overridden by [`animate_drops`]
/// (which runs after), so this is its landing spot once the arc finishes.
fn place_drops(mut drops: Query<(&Position, &AreaTag, &mut Transform), With<DroppedItem>>) {
    for (position, tag, mut transform) in &mut drops {
        *transform = sprite_transform(position.pos, drop_z(tag, position.pos));
    }
}

/// Arcs a freshly dropped item out from its origin, then on landing thuds with its drop sound and
/// stops animating (handing the rest position back to [`place_drops`]).
fn animate_drops(
    time: Res<Time>,
    mut drops: Query<(Entity, &Position, &AreaTag, &mut DropAnim, &mut Transform)>,
    mut play: MessageWriter<PlaySfx>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, position, tag, mut anim, mut transform) in &mut drops {
        anim.elapsed += dt;
        let t = ((anim.elapsed - anim.delay) / DROP_DURATION).clamp(0.0, 1.0);
        let ground = anim.from.lerp(anim.to, ease_out(t));
        let hop = DROP_HOP * (std::f32::consts::PI * t).sin();
        transform.translation = Vec3::new(ground.x, ground.y + hop, drop_z(tag, position.pos));
        if anim.elapsed - anim.delay >= DROP_DURATION {
            if let Some(id) = &anim.drop_sfx {
                play.write(PlaySfx {
                    id: id.clone(),
                    at: position.pos,
                });
            }
            commands.entity(entity).remove::<DropAnim>();
        }
    }
}

fn drop_z(tag: &AreaTag, pos: Pos<Tiles>) -> f32 {
    area::get(tag.area).map_or(0.0, |area| {
        dynamic_z(area.size.height, area.dynamic_layer() as f32, Tiles(pos.y))
    })
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
