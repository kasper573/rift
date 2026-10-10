use std::collections::HashMap;

use crate::core::assets::AssetService;
use crate::core::interpolate::{Interpolate, InterpolatePlugin};
use crate::core::math::Direction;
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds};
use crate::systems::actor::{self, Action, Actor, ActorModel};
use crate::systems::area::{self, AreaTag};
use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{Animator, atlas_rect, dynamic_z, sprite_transform};
use crate::systems::movement::RenderPosition;

pub struct ActorPlugin;

impl Plugin for ActorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>()
            .add_plugins(InterpolatePlugin::<RenderActor>::default())
            .add_systems(
                Update,
                (
                    attach_sprites,
                    (sync_actors, actor_cues).run_if(in_state(crate::systems::scene::Scene::Area)),
                )
                    .chain()
                    .in_set(WorldViewSystems),
            );
    }
}

pub const ACTOR_ANCHOR: Anchor = Anchor(Vec2::new(0.0, -1.0 / 6.0));

pub fn draw_actor_frame(
    sprite: &mut Sprite,
    model: &ActorModel,
    action: Action,
    dir: Direction,
    elapsed: Seconds,
    attack_rate: PlaybackRate,
) {
    let region = model.frame(action, dir, elapsed, attack_rate);
    let drawn = model.drawn_size(region);
    sprite.rect = Some(atlas_rect(region));
    sprite.custom_size = Some(Vec2::new(drawn.width, drawn.height));
}

/// The facing and animation to render an actor with. The server recomputes these alongside its
/// movement every tick, so they ride the same replication cadence as [`RenderPosition`]: the legs go
/// idle, and the heading turns, as the on-screen motion stops or changes course — not the snapshot
/// earlier they would if read live, which reads as the actor sliding or drifting. Discrete values,
/// so they switch as their segment starts rather than blending.
#[derive(Component, Clone, Copy, PartialEq)]
struct RenderActor {
    dir: Direction,
    action: Action,
}

impl Interpolate for RenderActor {
    type Source = Actor;

    fn sample(actor: &Actor) -> RenderActor {
        RenderActor {
            dir: actor.dir,
            action: actor.action,
        }
    }
}

fn attach_sprites(
    actors: Query<(Entity, &Actor), Without<Sprite>>,
    assets: Res<AssetServer>,
    service: Res<AssetService>,
    mut commands: Commands,
) {
    for (entity, actor) in &actors {
        let image = assets.load(actor::model(&service, actor.model).sheet().to_owned());
        commands.entity(entity).insert((
            Sprite { image, ..default() },
            ACTOR_ANCHOR,
            Transform::default(),
            Visibility::Hidden,
        ));
    }
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
        let model = actor::model(&service, actor.model);
        draw_actor_frame(
            &mut sprite,
            model,
            pose.action,
            pose.dir,
            elapsed,
            actor.attack_rate,
        );
        sprite.color = actor.color.color();
        let area = area::load(&service, tag.area);
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
        let since = (was == pose.action).then_some(then);
        let model = actor::model(&service, actor.model);
        let (cues, stepped) = model.cues(pose.action, pose.dir, since, now, actor.attack_rate);
        for id in cues {
            play.write(PlaySfx {
                id: *id,
                place: SfxPlace::World(at),
            });
        }
        if stepped && let Some(id) = area::load(&service, tag.area).tile_sfx_at(at.cell()) {
            play.write(PlaySfx {
                id: *id,
                place: SfxPlace::World(at),
            });
        }
    }
}
