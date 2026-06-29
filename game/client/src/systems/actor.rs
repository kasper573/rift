use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use world::core::assets::AssetService;
use world::core::tiling::{TilePos, Tiles};
use world::core::time::Seconds;
use world::systems::actor::{Action, Actor, Rgba, build_model};
use world::systems::area::{self, AreaTag};

use crate::core::render::{Animator, atlas_rect, dynamic_z, sprite_transform};
use crate::core::sfx::PlaySfx;
use crate::systems::interpolate::{RenderActor, RenderPosition};

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
    service: Res<AssetService>,
    mut commands: Commands,
) {
    let Ok(actor) = actors.get(add.entity) else {
        return;
    };
    let image = assets.load(
        service
            .resolve(*actor.model.get(), build_model)
            .sheet()
            .to_owned(),
    );
    commands.entity(add.entity).insert((
        Sprite { image, ..default() },
        Anchor(Vec2::new(0.0, -1.0 / 6.0)),
        Transform::default(),
        Visibility::Hidden,
    ));
}

type ActorView = (
    Entity,
    &'static Actor,
    &'static RenderPosition,
    &'static RenderActor,
    &'static AreaTag,
    &'static mut Sprite,
    &'static mut Transform,
    &'static mut Visibility,
);

fn sync_actors(
    time: Res<Time>,
    service: Res<AssetService>,
    mut animator: ResMut<Animator>,
    mut actors: Query<ActorView>,
) {
    let clock = Seconds(time.elapsed_secs());
    animator.retain(|entity| actors.contains(entity));
    for (entity, actor, render, pose, tag, mut sprite, mut transform, mut visibility) in &mut actors
    {
        let elapsed = animator.elapsed(entity, pose.action as u64, clock);
        let region = service.resolve(*actor.model.get(), build_model).frame(
            pose.action.name(),
            pose.dir,
            elapsed,
            actor.attack_rate,
        );
        sprite.rect = Some(atlas_rect(region));
        sprite.custom_size = Some(Vec2::new(region.size.width, region.size.height));
        sprite.color = rgba(actor.color);
        let area = service.resolve(tag.area.get().map, area::build_area);
        let at = render.0;
        *transform = sprite_transform(
            at,
            dynamic_z(area.size.height, area.dynamic_layer() as f32, Tiles(at.y)),
        );
        if *visibility == Visibility::Hidden {
            *visibility = Visibility::Inherited;
        }
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
    service: Res<AssetService>,
    mut animator: ResMut<Animator>,
    mut seen: ResMut<Seen>,
    actors: Query<(Entity, &Actor, &RenderPosition, &RenderActor, &AreaTag)>,
    mut play: MessageWriter<PlaySfx>,
) {
    let clock = Seconds(time.elapsed_secs());
    seen.0.retain(|entity, _| actors.contains(*entity));
    for (entity, actor, render, pose, tag) in &actors {
        let at = render.0;
        let now = animator.elapsed(entity, pose.action as u64, clock);
        let Some((was, then)) = seen.0.insert(entity, (pose.action, now)) else {
            continue;
        };
        let since = if was == pose.action {
            then
        } else {
            Seconds(-1.0)
        };
        let model = service.resolve(*actor.model.get(), build_model);
        let (cues, stepped) =
            model.cues(pose.action.name(), pose.dir, since, now, actor.attack_rate);
        for id in cues {
            play.write(PlaySfx { id: *id, at });
        }
        if stepped
            && let Some(id) = service
                .resolve(tag.area.get().map, area::build_area)
                .tile_sfx_at(at.cell())
        {
            play.write(PlaySfx { id: *id, at });
        }
    }
}
