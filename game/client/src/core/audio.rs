use std::collections::HashMap;

use bevy::prelude::*;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioSource, Decibels};
use world::core::math::{Pos, Size};
use world::core::tiling::Tiles;
use world::core::time::Seconds;

const HALF_VIEW: Size<Tiles> = Size::new(12.0, 9.0);
const STACK_WINDOW: Seconds = Seconds(0.1);

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_kira_audio::AudioPlugin)
            .init_resource::<Listener>()
            .init_resource::<SfxCatalog>()
            .init_resource::<Catalog>()
            .init_resource::<Played>()
            .add_message::<PlaySfx>()
            .add_systems(Startup, load)
            .add_systems(Update, mix);
    }
}

#[derive(Resource, Default)]
pub struct Listener(pub Option<Pos<Tiles>>);

#[derive(Message)]
pub struct PlaySfx {
    pub id: String,
    pub at: Pos<Tiles>,
}

#[derive(Resource, Default)]
pub struct SfxCatalog(pub Vec<SfxSpec>);

pub struct SfxSpec {
    pub id: String,
    pub path: String,
    pub volume: (f32, f32),
    pub pitch: (f32, f32),
}

#[derive(Resource, Default)]
struct Catalog(HashMap<String, Sound>);

struct Sound {
    handle: Handle<AudioSource>,
    volume: (f32, f32),
    pitch: (f32, f32),
    key: u64,
}

#[derive(Resource, Default)]
struct Played(HashMap<u64, Seconds>);

#[derive(Clone)]
struct Cue {
    proximity: f32,
    pan: f32,
    handle: Handle<AudioSource>,
    volume: (f32, f32),
    pitch: (f32, f32),
}

fn load(specs: Res<SfxCatalog>, assets: Res<AssetServer>, mut catalog: ResMut<Catalog>) {
    for (key, spec) in specs.0.iter().enumerate() {
        catalog.0.insert(
            spec.id.clone(),
            Sound {
                handle: assets.load(spec.path.clone()),
                volume: spec.volume,
                pitch: spec.pitch,
                key: key as u64,
            },
        );
    }
}

fn mix(
    mut requests: MessageReader<PlaySfx>,
    listener: Res<Listener>,
    time: Res<Time>,
    audio: Res<Audio>,
    catalog: Res<Catalog>,
    mut played: ResMut<Played>,
) {
    let Some(listener) = listener.0 else {
        requests.clear();
        return;
    };
    let clock = Seconds(time.elapsed_secs());
    let mut frame: HashMap<u64, Cue> = HashMap::new();
    for req in requests.read() {
        let Some(sound) = catalog.0.get(&req.id) else {
            continue;
        };
        let proximity = proximity_volume(listener, req.at);
        if proximity <= 0.0 {
            continue;
        }
        let cue = Cue {
            proximity,
            pan: proximity_pan(listener, req.at),
            handle: sound.handle.clone(),
            volume: sound.volume,
            pitch: sound.pitch,
        };
        let slot = frame.entry(sound.key).or_insert_with(|| cue.clone());
        if cue.proximity > slot.proximity {
            *slot = cue;
        }
    }
    for (key, cue) in frame {
        if !ready(&mut played.0, key, clock) {
            continue;
        }
        let volume = resolve(cue.volume, roll(clock, key)) * cue.proximity;
        let pitch = resolve(cue.pitch, roll(clock, key.wrapping_add(7)));
        audio
            .play(cue.handle)
            .with_volume(Decibels(20.0 * volume.max(1e-4).log10()))
            .with_playback_rate(f64::from(pitch))
            .with_panning(cue.pan);
    }
}

fn ready(played: &mut HashMap<u64, Seconds>, key: u64, clock: Seconds) -> bool {
    if played
        .get(&key)
        .is_some_and(|&last| clock - last < STACK_WINDOW)
    {
        return false;
    }
    played.insert(key, clock);
    true
}

fn resolve((min, max): (f32, f32), roll: f32) -> f32 {
    min + roll.clamp(0.0, 1.0) * (max - min)
}

fn roll(clock: Seconds, salt: u64) -> f32 {
    let bits = (clock.0.to_bits() as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    (bits % 1000) as f32 / 1000.0
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
