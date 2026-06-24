//! Emits the game's sound cues into the core mixer ([`crate::core::audio`]): actor animation cues
//! (plus the footstep tile sound) and item-consumed cues, and keeps the listener on the local player.
//! Loads the sound catalogue and resolves each cue's volume/pitch; the mixer handles spatialisation.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_kira_audio::prelude::AudioSource;
use world::core::math::Pos;
use world::core::tiling::{TilePos, Tiles};
use world::core::time::Seconds;
use world::systems::actor::{Action, Actor};
use world::systems::area::{self, Area, AreaTag};
use world::systems::items::ItemConsumed;
use world::systems::movement::{Position, position};
use world::systems::player::session;
use world::systems::sfx::sfx_table;

use crate::core::audio::{Listener, PlaySfx};
use crate::core::render::Animator;

pub struct CuePlugin;

impl Plugin for CuePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Seen>()
            .add_systems(Startup, load)
            .add_systems(
                Update,
                (set_listener, cues).run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

/// The loaded catalogue: a handle and a name→row index, mirroring `world`'s sfx table.
#[derive(Resource, Default)]
struct Catalog {
    sources: Vec<Handle<AudioSource>>,
    index: HashMap<String, usize>,
}

/// Each actor's last seen action and its elapsed time, so a cue fires once as the animation crosses it.
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

fn load(assets: Res<AssetServer>, mut commands: Commands) {
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

    // Sample each actor's animation clock and the action it was last seen in, before touching anything
    // else (the clock borrows the world mutably).
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
