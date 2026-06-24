//! The spatial sfx mixer: plays sounds positioned in the world, attenuated and panned relative to a
//! [`Listener`], with brief de-duplication so one sound can't stack on itself. Game systems load the
//! catalogue and emit [`PlaySfx`] anonymously — this knows nothing about what triggers a sound.

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
            .init_resource::<Played>()
            .add_message::<PlaySfx>()
            .add_systems(Update, mix);
    }
}

/// Where the player hears from; a game system keeps it on the local player. Sounds are attenuated and
/// panned relative to it, and nothing plays while it is absent.
#[derive(Resource, Default)]
pub struct Listener(pub Option<Pos<Tiles>>);

/// A request to play a sound at a world position. Any system emits these anonymously. `key` identifies
/// the sound so it can't restack within [`STACK_WINDOW`]; `volume`/`pitch` are the pre-attenuation
/// values the caller chose.
#[derive(Message)]
pub struct PlaySfx {
    pub sound: Handle<AudioSource>,
    pub at: Pos<Tiles>,
    pub volume: f32,
    pub pitch: f32,
    pub key: u64,
}

#[derive(Resource, Default)]
struct Played(HashMap<u64, Seconds>);

fn mix(
    mut requests: MessageReader<PlaySfx>,
    listener: Res<Listener>,
    time: Res<Time>,
    audio: Res<Audio>,
    mut played: ResMut<Played>,
) {
    let Some(listener) = listener.0 else {
        requests.clear();
        return;
    };
    let clock = Seconds(time.elapsed_secs());
    // Collapse this frame's requests to the loudest per sound, after spatial attenuation.
    let mut frame: HashMap<u64, (f32, f32, Handle<AudioSource>, f32)> = HashMap::new();
    for req in requests.read() {
        let volume = req.volume * proximity_volume(listener, req.at);
        if volume <= 0.0 {
            continue;
        }
        let pan = proximity_pan(listener, req.at);
        let slot = frame
            .entry(req.key)
            .or_insert((volume, pan, req.sound.clone(), req.pitch));
        if volume > slot.0 {
            *slot = (volume, pan, req.sound.clone(), req.pitch);
        }
    }
    for (key, (volume, pan, sound, pitch)) in frame {
        if !ready(&mut played.0, key, clock) {
            continue;
        }
        audio
            .play(sound)
            .with_volume(Decibels(20.0 * volume.max(1e-4).log10()))
            .with_playback_rate(f64::from(pitch))
            .with_panning(pan);
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

fn proximity_volume(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    let offset = source - listener;
    let dx = offset.x.abs() / HALF_VIEW.width;
    let dy = offset.y.abs() / HALF_VIEW.height;
    (1.0 - dx.max(dy)).clamp(0.0, 1.0)
}

fn proximity_pan(listener: Pos<Tiles>, source: Pos<Tiles>) -> f32 {
    ((source - listener).x / HALF_VIEW.width).clamp(-1.0, 1.0)
}
