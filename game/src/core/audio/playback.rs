use std::collections::HashMap;

use crate::core::assets::AssetRef;
use crate::core::audio::decibels;
use crate::core::audio::loader::AudioLoader;
use crate::core::audio::mix::{AudioCategory, AudioMix};
use crate::core::content::Content;
use crate::core::math::{Pos, Rng, Size};
use crate::core::tiling::Tiles;
use crate::core::time::{PlaybackRate, Seconds};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioSource};

const HALF_VIEW: Size<Tiles> = Size::new(12.0, 9.0);
const STACK_WINDOW: Seconds = Seconds(0.1);

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_kira_audio::AudioPlugin)
            .register_asset_loader(AudioLoader)
            .init_resource::<Listener>()
            .init_resource::<AudioMix>()
            .init_resource::<Catalog>()
            .init_resource::<Played>()
            .add_message::<PlaySfx>()
            .add_message::<PlayClip>()
            .add_systems(Startup, load)
            .add_systems(Update, (mix, play_clips));
    }
}

pub use crate::data::sfx::Id as SfxId;

#[derive(Clone)]
pub struct SfxDef {
    pub src: AssetRef,
    pub volume: SfxScalar,
    pub pitch: SfxScalar,
}

impl crate::core::content::ContentRow for SfxDef {
    const TABLE: &'static str = "sfx";
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxScalar {
    Fixed(f32),
    Random(f32, f32),
}

#[derive(Resource, Default)]
pub struct Listener(pub Option<Pos<Tiles>>);

#[derive(Message)]
pub struct PlaySfx {
    pub id: SfxId,
    pub place: SfxPlace,
}

#[derive(Message)]
pub struct PlayClip {
    pub clip: Handle<AudioSource>,
    pub category: AudioCategory,
    pub place: SfxPlace,
    pub tune: SfxTune,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfxTune {
    pub pitch: PlaybackRate,
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxPlace {
    World(Pos<Tiles>),
    Interface,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionalAudio {
    pub proximity: f32,
    pub pan: f32,
}

impl Default for SfxScalar {
    fn default() -> SfxScalar {
        SfxScalar::Fixed(1.0)
    }
}

impl SfxScalar {
    pub fn resolve(self, rng: &mut Rng) -> f32 {
        match self {
            SfxScalar::Fixed(value) => value,
            SfxScalar::Random(min, max) => min + rng.rand_float() * (max - min),
        }
    }
}

impl PositionalAudio {
    pub const CENTERED: PositionalAudio = PositionalAudio {
        proximity: 1.0,
        pan: 0.0,
    };

    pub fn of(listener: Pos<Tiles>, source: Pos<Tiles>, reach: Size<Tiles>) -> PositionalAudio {
        let offset = source - listener;
        let dx = offset.x.abs() / reach.width;
        let dy = offset.y.abs() / reach.height;
        PositionalAudio {
            proximity: (1.0 - dx.max(dy)).clamp(0.0, 1.0),
            pan: (offset.x / HALF_VIEW.width).clamp(-1.0, 1.0),
        }
    }

    pub fn audible(self) -> bool {
        self.proximity > 0.0
    }
}

#[derive(Resource, Default)]
struct Catalog(Vec<Sound>);

struct Sound {
    handle: Handle<AudioSource>,
    volume: SfxScalar,
    pitch: SfxScalar,
}

#[derive(Resource, Default)]
struct Played(HashMap<SfxId, Seconds>);

#[derive(Clone)]
struct Cue {
    proximity: f32,
    pan: f32,
    handle: Handle<AudioSource>,
    volume: SfxScalar,
    pitch: SfxScalar,
}

fn load(assets: Res<AssetServer>, content: Res<Content>, mut catalog: ResMut<Catalog>) {
    catalog.0 = content
        .table::<SfxDef>()
        .rows()
        .iter()
        .map(|def| Sound {
            handle: assets.load(def.src.0),
            volume: def.volume,
            pitch: def.pitch,
        })
        .collect();
}

fn mix(
    mut requests: MessageReader<PlaySfx>,
    listener: Res<Listener>,
    time: Res<Time>,
    speakers: Speakers,
    catalog: Res<Catalog>,
    mut played: ResMut<Played>,
    mut rng: ResMut<Rng>,
) {
    let clock = Seconds(time.elapsed_secs());
    let mut frame: HashMap<SfxId, Cue> = HashMap::new();
    for req in requests.read() {
        let Some(sound) = catalog.0.get(req.id.index()) else {
            continue;
        };
        let Some(PositionalAudio { proximity, pan }) = heard_at(req.place, listener.0) else {
            continue;
        };
        let cue = Cue {
            proximity,
            pan,
            handle: sound.handle.clone(),
            volume: sound.volume,
            pitch: sound.pitch,
        };
        let slot = frame.entry(req.id).or_insert_with(|| cue.clone());
        if cue.proximity > slot.proximity {
            *slot = cue;
        }
    }
    for (id, cue) in frame {
        if !ready(&mut played.0, id, clock) {
            continue;
        }
        let tune = SfxTune {
            pitch: PlaybackRate(cue.pitch.resolve(&mut rng)),
            volume: cue.volume.resolve(&mut rng) * cue.proximity,
        };
        speakers.play(cue.handle, AudioCategory::Effects, tune, cue.pan);
    }
}

fn play_clips(mut clips: MessageReader<PlayClip>, listener: Res<Listener>, speakers: Speakers) {
    for clip in clips.read() {
        let Some(PositionalAudio { proximity, pan }) = heard_at(clip.place, listener.0) else {
            continue;
        };
        let tune = SfxTune {
            volume: clip.tune.volume * proximity,
            ..clip.tune
        };
        speakers.play(clip.clip.clone(), clip.category, tune, pan);
    }
}

#[derive(SystemParam)]
struct Speakers<'w> {
    audio: Res<'w, Audio>,
    audio_mix: Res<'w, AudioMix>,
}

impl Speakers<'_> {
    fn play(&self, clip: Handle<AudioSource>, category: AudioCategory, tune: SfxTune, pan: f32) {
        let volume = tune.volume * self.audio_mix.gain(category);
        if volume <= 0.0 {
            return;
        }
        self.audio
            .play(clip)
            .with_volume(decibels(volume))
            .with_playback_rate(f64::from(tune.pitch.0))
            .with_panning(pan);
    }
}

fn heard_at(place: SfxPlace, listener: Option<Pos<Tiles>>) -> Option<PositionalAudio> {
    let heard = match (place, listener) {
        (SfxPlace::Interface, _) => PositionalAudio::CENTERED,
        (SfxPlace::World(at), Some(listener)) => PositionalAudio::of(listener, at, HALF_VIEW),
        (SfxPlace::World(_), None) => return None,
    };
    heard.audible().then_some(heard)
}

fn ready(played: &mut HashMap<SfxId, Seconds>, id: SfxId, clock: Seconds) -> bool {
    if played
        .get(&id)
        .is_some_and(|&last| clock - last < STACK_WINDOW)
    {
        return false;
    }
    played.insert(id, clock);
    true
}
