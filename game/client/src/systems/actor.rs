//! Presents replicated actors: their sprites (sampled from the model's animation, tinted, depth-sorted)
//! and their sound cues (animation cues, footsteps, item use) emitted into the core audio mixer. Also
//! keeps the listener and the world camera on the local player. The generic render/audio engines live
//! in `crate::core`; this is where the game plugs into them.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::window::PrimaryWindow;
use bevy_kira_audio::prelude::AudioSource;
use world::core::math::Pos;
use world::core::table::Id;
use world::core::tiling::{TilePos, TileSize, Tiles};
use world::core::time::Seconds;
use world::systems::actor::{Action, Actor, Rgba};
use world::systems::area::{self, Area, AreaDef, AreaTag};
use world::systems::items::ItemConsumed;
use world::systems::movement::{Position, position};
use world::systems::player::Owner;
use world::systems::player::session::{self, MyClient};
use world::systems::sfx::sfx_table;

use crate::core::audio::{Listener, PlaySfx};
use crate::core::render::camera::WorldCamera;
use crate::core::render::present::target_size;
use crate::core::render::screen::ToScreen;
use crate::core::render::{Animator, TILE, atlas_rect, dynamic_z, sprite_transform};

pub struct ActorPlugin;

impl Plugin for ActorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>()
            .add_observer(attach_sprite)
            .add_systems(Startup, load_sfx)
            .add_systems(
                Update,
                (sync_actors, follow_camera, set_listener, cues)
                    .run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

// --- sprites -------------------------------------------------------------------------------------

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
        let Some(area) = area::areas().get(tag.area.index()) else {
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

// --- camera --------------------------------------------------------------------------------------

fn follow_camera(
    me: Res<MyClient>,
    players: Query<(&Owner, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    let Some(center) = camera_center(position.pos, tag.area, view_half(&window)) else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        let p = center.to_screen();
        transform.translation.x = p.x;
        transform.translation.y = p.y;
    }
}

fn camera_center(at: Pos<Tiles>, area_id: Id<AreaDef>, half: Vec2) -> Option<Pos<Tiles>> {
    let area = area::areas().get(area_id.index())?;
    let bounds = area.size.bounds();
    let lo = Pos::new(bounds.min().x + half.x, bounds.min().y + half.y);
    let hi = Pos::new(
        (bounds.max().x - half.x).max(lo.x),
        (bounds.max().y - half.y).max(lo.y),
    );
    Some(snap(at.clamp(lo, hi)))
}

fn view_half(window: &Window) -> Vec2 {
    let (w, h) = target_size(window);
    Vec2::new(0.5 * w as f32 / TILE.0, 0.5 * h as f32 / TILE.0)
}

fn snap(p: Pos<Tiles>) -> Pos<Tiles> {
    let axis = |t: f32| (t * TILE.0).round() / TILE.0;
    Pos::new(axis(p.x), axis(p.y))
}

// --- sound cues ----------------------------------------------------------------------------------

/// The loaded sound catalogue: a handle and a name→row index, mirroring `world`'s sfx table.
#[derive(Resource, Default)]
struct Catalog {
    sources: Vec<Handle<AudioSource>>,
    index: HashMap<String, usize>,
}

/// Each actor's last seen action and elapsed time, so a cue fires once as the animation crosses it.
#[derive(Resource, Default)]
struct Seen(HashMap<Entity, (Action, Seconds)>);

/// One actor sampled for this frame's cues: its current animation time and the action/time it was last
/// seen in (absent the first frame it appears).
struct Sampled {
    actor: Actor,
    at: Pos<Tiles>,
    now: Seconds,
    prev: Option<(Action, Seconds)>,
}

fn load_sfx(assets: Res<AssetServer>, mut commands: Commands) {
    let mut catalog = Catalog::default();
    for (row, def) in sfx_table().iter().enumerate() {
        catalog.sources.push(assets.load(def.src.clone()));
        catalog.index.insert(def.id.0.clone(), row);
    }
    commands.insert_resource(catalog);
}

fn set_listener(world: &mut World) {
    let at = session::me(world)
        .and_then(|me| me.get::<Position>())
        .map(|position| position.pos);
    world.resource_mut::<Listener>().0 = at;
}

fn cues(world: &mut World) {
    let clock = Seconds(world.resource::<Time>().elapsed_secs());
    let area: Option<&'static Area> = session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()));

    let actors: Vec<(Entity, Actor, Pos<Tiles>)> = world
        .query::<(Entity, &Actor, &Position)>()
        .iter(world)
        .map(|(entity, actor, position)| (entity, actor.clone(), position.pos))
        .collect();

    let mut seen = world.remove_resource::<Seen>().expect("seen");
    seen.0
        .retain(|entity, _| actors.iter().any(|(e, ..)| e == entity));

    let mut animator = world.resource_mut::<Animator>();
    let sampled: Vec<Sampled> = actors
        .iter()
        .map(|(entity, actor, source)| {
            let now = animator.elapsed(*entity, actor.action as u64, clock);
            let prev = seen.0.insert(*entity, (actor.action, now));
            Sampled {
                actor: actor.clone(),
                at: *source,
                now,
                prev,
            }
        })
        .collect();

    let mut requests: Vec<PlaySfx> = Vec::new();
    {
        let catalog = world.resource::<Catalog>();
        for s in &sampled {
            let Some((was, then)) = s.prev else {
                continue;
            };
            let prev = if was == s.actor.action {
                then
            } else {
                Seconds(-1.0)
            };
            let model = s.actor.model.get();
            let (cues, stepped) = model.cues(
                s.actor.action.name(),
                s.actor.dir,
                prev,
                s.now,
                s.actor.attack_rate,
            );
            for id in cues {
                push(catalog, &mut requests, &id.0, s.at, clock);
            }
            if let (Some(area), true) = (area, stepped)
                && let Some(id) = area.tile_sfx_at(s.at.cell())
            {
                push(catalog, &mut requests, &id.0, s.at, clock);
            }
        }
    }

    let consumed: Vec<ItemConsumed> = world
        .resource_mut::<Messages<ItemConsumed>>()
        .drain()
        .collect();
    for item in &consumed {
        let Some(id) = item.item.get().sfx.as_ref() else {
            continue;
        };
        let Some(source) = position(world, item.actor) else {
            continue;
        };
        push(
            world.resource::<Catalog>(),
            &mut requests,
            &id.0,
            source,
            clock,
        );
    }

    let mut writer = world.resource_mut::<Messages<PlaySfx>>();
    for request in requests {
        writer.write(request);
    }
    world.insert_resource(seen);
}

fn push(catalog: &Catalog, out: &mut Vec<PlaySfx>, id: &str, at: Pos<Tiles>, clock: Seconds) {
    let Some(&row) = catalog.index.get(id) else {
        return;
    };
    let def = &sfx_table()[row];
    out.push(PlaySfx {
        sound: catalog.sources[row].clone(),
        at,
        volume: def.volume.resolve(fastrand_unit(clock, row)),
        pitch: def.pitch.resolve(fastrand_unit(clock, row.wrapping_add(7))),
        key: row as u64,
    });
}

fn fastrand_unit(clock: Seconds, salt: usize) -> f32 {
    let bits = (clock.0.to_bits() as usize)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    (bits % 1000) as f32 / 1000.0
}
