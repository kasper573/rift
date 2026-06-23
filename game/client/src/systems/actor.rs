//! Renders replicated [`Actor`]s: attaches a sprite when one appears, then each frame samples its
//! model's animation (timed by the shared [`Animator`]), tints it, and depth-sorts it within its area.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::tiling::Tiles;
use world::core::time::Seconds;
use world::systems::actor::{Actor, Rgba};
use world::systems::area;
use world::systems::area::AreaTag;
use world::systems::movement::Position;

use crate::core::render::{Animator, atlas_rect, dynamic_z, sprite_transform};

pub struct ActorPlugin;

impl Plugin for ActorPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(attach_sprite).add_systems(
            Update,
            sync_actors.run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

fn attach_sprite(
    add: On<Add, Actor>,
    actors: Query<&Actor>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(actor) = actors.get(add.entity) else {
        return;
    };
    let image = assets.load(actor.model.get().sheet().to_owned());
    commands.entity(add.entity).insert((
        Sprite { image, ..default() },
        Anchor(Vec2::new(0.0, -1.0 / 6.0)),
        Transform::default(),
        Visibility::default(),
    ));
}

fn sync_actors(
    time: Res<Time>,
    mut animator: ResMut<Animator>,
    mut actors: Query<(
        Entity,
        &Actor,
        &Position,
        &AreaTag,
        &mut Sprite,
        &mut Transform,
    )>,
) {
    let clock = Seconds(time.elapsed_secs());
    animator.retain(|entity| actors.contains(entity));
    for (entity, actor, position, tag, mut sprite, mut transform) in &mut actors {
        let elapsed = animator.elapsed(entity, actor.action, clock);
        let region =
            actor
                .model
                .get()
                .frame(actor.action.name(), actor.dir, elapsed, actor.attack_rate);
        sprite.rect = Some(atlas_rect(region));
        sprite.custom_size = Some(Vec2::new(region.size.width, region.size.height));
        sprite.color = rgba(actor.color);
        let Some(area) = area::areas().get(tag.area.index()) else {
            continue;
        };
        *transform = sprite_transform(
            position.pos,
            dynamic_z(area, area.dynamic_layer() as f32, Tiles(position.pos.y)),
        );
    }
}

fn rgba(tint: Rgba) -> Color {
    let [r, g, b, a] = tint.0.to_be_bytes();
    Color::srgba_u8(r, g, b, a)
}
