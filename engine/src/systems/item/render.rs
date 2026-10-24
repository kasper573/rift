use bevy::prelude::*;
use ui::motion;

use crate::core::audio::playback::{PlaySfx, SfxId, SfxPlace};
use crate::core::content::Content;
use crate::core::math::{Pos, WorldPx};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{RowDepth, ToScreen, dynamic_z, place_sprite};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::systems::area::{self, AreaTag};
use crate::systems::item::{DroppedItem, ItemConsumed, ItemsDropped};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::screen::ClientScreen;

const DROP_SIZE: WorldPx = WorldPx(12.0);
const DROP_STAGGER: Seconds = Seconds(0.06);
const DROP_DURATION: Seconds = Seconds(0.5);
const DROP_HOP: WorldPx = WorldPx(24.0);

pub struct ItemRenderPlugin;

impl Plugin for ItemRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(attach_drop_sprite).add_systems(
            Update,
            (
                use_sounds,
                (start_drops, place_drops, animate_drops)
                    .chain()
                    .in_set(WorldViewSystems),
            )
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

#[derive(Component)]
struct DropAnim {
    from: Vec2,
    to: Vec2,
    delay: Seconds,
    elapsed: Seconds,
    drop_sfx: SfxId,
}

fn use_sounds(
    content: Res<Content>,
    mut consumed: MessageReader<ItemConsumed>,
    positions: Query<&RenderPosition>,
    mut play: MessageWriter<PlaySfx>,
) {
    for consumed in consumed.read() {
        let Some(id) = consumed.item.get(&content).sfx.on_use else {
            continue;
        };
        let Ok(position) = positions.get(consumed.actor) else {
            continue;
        };
        play.write(PlaySfx {
            id,
            place: SfxPlace::World(position.0),
        });
    }
}

/// An item lying in the world: its icon, drawn small on the ground.
pub fn drop_sprite(icon: Handle<Image>) -> Sprite {
    Sprite {
        image: icon,
        custom_size: Some(Vec2::splat(DROP_SIZE.0)),
        ..default()
    }
}

fn attach_drop_sprite(
    add: On<Add<DroppedItem>>,
    drops: Query<&DroppedItem>,
    content: Res<Content>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(dropped) = drops.get(add.entity) else {
        return;
    };
    let image = assets.load(dropped.item.get(&content).icon.path());
    commands.entity(add.entity).insert((
        drop_sprite(image),
        Transform::default(),
        RowDepth::default(),
        Visibility::default(),
    ));
}

fn start_drops(
    mut dropped: MessageReader<ItemsDropped>,
    content: Res<Content>,
    drops: Query<(&DroppedItem, &Position)>,
    mut commands: Commands,
) {
    for dropped in dropped.read() {
        for (index, &entity) in dropped.items.iter().enumerate() {
            let Ok((item, position)) = drops.get(entity) else {
                continue;
            };
            commands.entity(entity).try_insert(DropAnim {
                from: dropped.from.to_screen(),
                to: position.pos.to_screen(),
                delay: DROP_STAGGER * index as f32,
                elapsed: Seconds(0.0),
                drop_sfx: item.item.get(&content).sfx.drop,
            });
        }
    }
}

fn place_drops(
    content: Res<Content>,
    mut drops: Query<(&Position, &AreaTag, &mut Transform, &mut RowDepth), With<DroppedItem>>,
) {
    for (position, tag, mut transform, mut depth) in &mut drops {
        *depth = RowDepth(drop_z(&content, tag, position.pos));
        place_sprite(&mut transform, position.pos);
    }
}

fn animate_drops(
    time: Res<Time>,
    mut drops: Query<(Entity, &Position, &mut DropAnim, &mut Transform)>,
    mut play: MessageWriter<PlaySfx>,
    mut commands: Commands,
) {
    let dt = Seconds(time.delta_secs());
    for (entity, position, mut anim, mut transform) in &mut drops {
        anim.elapsed += dt;
        let t = (anim.elapsed - anim.delay)
            .ratio(DROP_DURATION)
            .clamp(0.0, 1.0);
        let ground = anim.from.lerp(anim.to, motion::ease_out(t));
        let hop = DROP_HOP.0 * (std::f32::consts::PI * t).sin();
        transform.translation.x = ground.x;
        transform.translation.y = ground.y + hop;
        if anim.elapsed - anim.delay >= DROP_DURATION {
            play.write(PlaySfx {
                id: anim.drop_sfx,
                place: SfxPlace::World(position.pos),
            });
            commands.entity(entity).remove::<DropAnim>();
        }
    }
}

fn drop_z(content: &Content, tag: &AreaTag, pos: Pos<Meters>) -> f32 {
    let area = area::load(content, tag.area);
    dynamic_z(area.size.height, area.sort_base(), Meters(pos.y))
}
