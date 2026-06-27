use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::tiling::{TilePos, Tiles};
use world::core::time::Seconds;
use world::systems::actor::{Action, Actor, Rgba};
use world::systems::area::{self, AreaTag};
use world::systems::movement::Position;

use crate::core::audio::PlaySfx;
use crate::core::render::{Animator, atlas_rect, dynamic_z, sprite_transform};

pub struct ActorPlugin;

impl Plugin for ActorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>()
            .add_observer(attach_sprite)
            .add_systems(
                Update,
                (sync_actors, actor_cues).run_if(in_state(crate::Scene::Area)),
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
        let elapsed = animator.elapsed(entity, actor.action as u64, clock);
        let region =
            actor
                .model
                .get()
                .frame(actor.action.name(), actor.dir, elapsed, actor.attack_rate);
        sprite.rect = Some(atlas_rect(region));
        sprite.custom_size = Some(Vec2::new(region.size.width, region.size.height));
        sprite.color = rgba(actor.color);
        let Some(area) = area::get(tag.area) else {
            continue;
        };
        *transform = sprite_transform(
            position.pos,
            dynamic_z(
                area.size.height,
                area.dynamic_layer() as f32,
                Tiles(position.pos.y),
            ),
        );
    }
}

fn rgba(tint: Rgba) -> Color {
    let [r, g, b, a] = tint.0.to_be_bytes();
    Color::srgba_u8(r, g, b, a)
}

#[derive(Resource, Default)]
struct Seen(HashMap<Entity, (Action, Seconds)>);

fn actor_cues(
    time: Res<Time>,
    mut animator: ResMut<Animator>,
    mut seen: ResMut<Seen>,
    actors: Query<(Entity, &Actor, &Position, &AreaTag)>,
    mut play: MessageWriter<PlaySfx>,
) {
    let clock = Seconds(time.elapsed_secs());
    seen.0.retain(|entity, _| actors.contains(*entity));
    for (entity, actor, position, tag) in &actors {
        let now = animator.elapsed(entity, actor.action as u64, clock);
        let Some((was, then)) = seen.0.insert(entity, (actor.action, now)) else {
            continue;
        };
        let since = if was == actor.action {
            then
        } else {
            Seconds(-1.0)
        };
        let model = actor.model.get();
        let (cues, stepped) = model.cues(
            actor.action.name(),
            actor.dir,
            since,
            now,
            actor.attack_rate,
        );
        for id in cues {
            play.write(PlaySfx {
                id: id.0.clone(),
                at: position.pos,
            });
        }
        if stepped
            && let Some(area) = area::get(tag.area)
            && let Some(id) = area.tile_sfx_at(position.pos.cell())
        {
            play.write(PlaySfx {
                id: id.0.clone(),
                at: position.pos,
            });
        }
    }
}
