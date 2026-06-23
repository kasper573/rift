use std::collections::HashMap;

use bevy::prelude::*;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioSource, Decibels};
use world::core::math::{Pos, Size};
use world::core::tiling::{TilePos, Tiles};
use world::core::time::Seconds;
use world::systems::actor::{Action, Actor};
use world::systems::area;
use world::systems::area::AreaTag;
use world::systems::items::ItemConsumed;
use world::systems::movement::{Position, position};
use world::systems::player::session;
use world::systems::sfx::sfx_table;

use crate::GameScene;
use crate::core::render::Animator;

const HALF_VIEW: Size<Tiles> = Size::new(12.0, 9.0);
const STACK_WINDOW: Seconds = Seconds(0.1);

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_kira_audio::AudioPlugin)
            .add_systems(Startup, load)
            .add_systems(Update, play_cues.run_if(in_state(GameScene::Playing)));
    }
}

#[derive(Resource, Default)]
struct Sfx {
    sources: Vec<Handle<AudioSource>>,
    index: HashMap<String, usize>,
    seen: HashMap<Entity, (Action, Seconds)>,
    played: HashMap<usize, Seconds>,
}

fn load(assets: Res<AssetServer>, mut commands: Commands) {
    let mut sfx = Sfx::default();
    for (row, def) in sfx_table().iter().enumerate() {
        sfx.sources.push(assets.load(def.src.clone()));
        sfx.index.insert(def.id.0.clone(), row);
    }
    commands.insert_resource(sfx);
}

fn play_cues(world: &mut World) {
    let Some(listener) = session::me(world)
        .and_then(|me| me.get::<Position>())
        .map(|position| position.pos)
    else {
        return;
    };
    let clock = Seconds(world.resource::<Time>().elapsed_secs());
    let area = session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()));
    let mut sfx = world.remove_resource::<Sfx>().expect("sfx loaded");

    let mut frame: HashMap<usize, (f32, f32)> = HashMap::new();
    let actors: Vec<(Entity, Actor, Pos<Tiles>)> = world
        .query::<(Entity, &Actor, &Position)>()
        .iter(world)
        .map(|(entity, actor, position)| (entity, actor.clone(), position.pos))
        .collect();
    sfx.seen
        .retain(|entity, _| actors.iter().any(|(e, ..)| e == entity));

    let mut animator = world.resource_mut::<Animator>();
    for (entity, actor, source) in &actors {
        let now = animator.elapsed(*entity, actor.action, clock);
        let Some((was, then)) = sfx.seen.insert(*entity, (actor.action, now)) else {
            continue;
        };
        let volume = proximity_volume(listener, *source);
        if volume <= 0.0 {
            continue;
        }
        let pan = proximity_pan(listener, *source);
        let prev = if was == actor.action {
            then
        } else {
            Seconds(-1.0)
        };
        let model = actor.model.get();
        let (cues, stepped) =
            model.cues(actor.action.name(), actor.dir, prev, now, actor.attack_rate);
        for id in cues {
            collect(&sfx.index, &mut frame, &id.0, volume, pan);
        }
        if let (Some(area), true) = (area, stepped)
            && let Some(id) = area.tile_sfx_at(source.cell())
        {
            collect(&sfx.index, &mut frame, &id.0, volume, pan);
        }
    }

    for consumed in world
        .resource_mut::<Messages<ItemConsumed>>()
        .drain()
        .collect::<Vec<_>>()
    {
        item_cue(world, &sfx.index, &mut frame, listener, &consumed);
    }

    let audio = world.resource::<Audio>();
    for (row, (volume, pan)) in frame {
        if !ready(&mut sfx.played, row, clock) {
            continue;
        }
        let def = &sfx_table()[row];
        let base = def.volume.resolve(fastrand_unit(clock, row));
        let pitch = def.pitch.resolve(fastrand_unit(clock, row.wrapping_add(7)));
        audio
            .play(sfx.sources[row].clone())
            .with_volume(Decibels(20.0 * (base * volume).max(1e-4).log10()))
            .with_playback_rate(f64::from(pitch))
            .with_panning(pan);
    }

    world.insert_resource(sfx);
}

fn item_cue(
    world: &World,
    index: &HashMap<String, usize>,
    frame: &mut HashMap<usize, (f32, f32)>,
    listener: Pos<Tiles>,
    consumed: &ItemConsumed,
) {
    let Some(id) = consumed.item.get().sfx.as_ref() else {
        return;
    };
    let Some(source) = position(world, consumed.actor) else {
        return;
    };
    let volume = proximity_volume(listener, source);
    if volume > 0.0 {
        collect(index, frame, &id.0, volume, proximity_pan(listener, source));
    }
}

fn collect(
    index: &HashMap<String, usize>,
    frame: &mut HashMap<usize, (f32, f32)>,
    id: &str,
    volume: f32,
    pan: f32,
) {
    let Some(&row) = index.get(id) else {
        return;
    };
    let slot = frame.entry(row).or_insert((0.0, 0.0));
    if volume > slot.0 {
        *slot = (volume, pan);
    }
}

fn ready(played: &mut HashMap<usize, Seconds>, row: usize, clock: Seconds) -> bool {
    if played
        .get(&row)
        .is_some_and(|&last| clock - last < STACK_WINDOW)
    {
        return false;
    }
    played.insert(row, clock);
    true
}

fn proximity_volume(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    let offset = source - listener;
    let dx = offset.x.abs() / HALF_VIEW.width;
    let dy = offset.y.abs() / HALF_VIEW.height;
    (1.0 - dx.max(dy)).clamp(0.0, 1.0)
}

fn proximity_pan(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    ((source - listener).x / HALF_VIEW.width).clamp(-1.0, 1.0)
}

fn fastrand_unit(clock: Seconds, salt: usize) -> f32 {
    let bits = (clock.0.to_bits() as usize)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    (bits % 1000) as f32 / 1000.0
}
