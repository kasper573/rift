use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_kira_audio::prelude::{
    Audio, AudioApp, AudioChannel, AudioControl, AudioInstance, AudioSource, AudioTween,
    PlaybackState,
};
use kira::StartTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AudioRef;
use crate::core::audio::decibels;
use crate::core::audio::loader::AudioLoader;
use crate::core::audio::mix::{AudioCategory, AudioMix};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, FileWriter, Nearby, PartSpec, Parts, Shape,
};
use crate::core::math::{Pos, Rng, Size};
use crate::core::time::{PlaybackRate, Seconds};
use crate::core::units::Meters;

const HALF_VIEW: Size<Meters> = Size::new(12.0, 9.0);
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
            .init_resource::<Sounding>()
            .add_audio_channel::<VoiceTrack>()
            .add_message::<PlaySfx>()
            .add_message::<PlayClip>()
            .add_message::<HushVoice>()
            .add_systems(
                Update,
                (
                    load.run_if(resource_exists_and_changed::<Content>),
                    mix,
                    hush,
                    play_clips,
                )
                    .chain(),
            );
    }
}

pub type SfxId = ContentId<SfxDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SfxDef {
    #[serde(skip)]
    pub file: AudioRef,
    pub volume: SfxScalar,
    pub pitch: SfxScalar,
}

pub const SFX_CLIP: PartSpec = PartSpec::File {
    name: "clip",
    exts: &["wav", "flac"],
};

impl ContentFile for SfxDef {
    const TABLE: &'static str = "sfx";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(_: &Nearby) -> Result<SfxDef, String> {
        Ok(SfxDef {
            file: AudioRef::default(),
            volume: SfxScalar::default(),
            pitch: SfxScalar::default(),
        })
    }
}

impl Bundle for SfxDef {
    const PARTS: &'static [PartSpec] = &[SFX_CLIP];

    fn read_parts(&mut self, parts: &mut Parts) {
        if let Some(clip) = parts.required(SFX_CLIP) {
            self.file = AudioRef::new(parts.path(clip));
        }
    }

    fn write_parts(&self, _: &str, _: &mut FileWriter) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum SfxScalar {
    Fixed(f32),
    Random { min: f32, max: f32 },
}

#[derive(Resource, Default)]
pub struct Listener(pub Option<Pos<Meters>>);

#[derive(Message)]
pub struct PlaySfx {
    pub id: SfxId,
    pub place: SfxPlace,
}

/// A clip to sound at `due` on the app's clock (`Time::elapsed`): asked for ahead of its moment,
/// it starts on that moment whatever the frame rate, since the audio engine holds it until then.
#[derive(Message)]
pub struct PlayClip {
    pub clip: Handle<AudioSource>,
    pub category: AudioCategory,
    pub place: SfxPlace,
    pub tune: SfxTune,
    pub due: Duration,
}

/// Silences the voice, clips sounding and clips due alike, so a line cut short stops at once.
#[derive(Message)]
pub struct HushVoice;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfxTune {
    pub pitch: PlaybackRate,
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxPlace {
    World(Pos<Meters>),
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
            SfxScalar::Random { min, max } => min + rng.rand_float() * (max - min),
        }
    }
}

impl PositionalAudio {
    pub const CENTERED: PositionalAudio = PositionalAudio {
        proximity: 1.0,
        pan: 0.0,
    };

    pub fn of(listener: Pos<Meters>, source: Pos<Meters>, reach: Size<Meters>) -> PositionalAudio {
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

/// Asks for an sfx as a `PlaySfx` message does, and counts it as sounding from this moment, so
/// whoever asked sees it sound before the mixer has reached the request.
pub fn play(world: &mut World, request: PlaySfx) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    world
        .resource_mut::<Sounding>()
        .0
        .entry(request.id)
        .or_default()
        .push(Voice {
            instance: None,
            asked: now,
            begun: false,
        });
    world.write_message(request);
}

/// Whether an sfx asked for still sounds.
pub fn is_sounding(world: &mut World, id: SfxId) -> bool {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    world.resource_scope(|world, mut sounding: Mut<Sounding>| {
        sounding.settle(world.resource::<Assets<AudioInstance>>(), now);
        sounding.0.contains_key(&id)
    })
}

/// Stops every instance of an sfx still sounding.
pub fn stop(world: &mut World, id: SfxId) {
    world.resource_scope(|world, mut sounding: Mut<Sounding>| {
        let mut instances = world.resource_mut::<Assets<AudioInstance>>();
        for voice in sounding.0.remove(&id).unwrap_or_default() {
            if let Some(mut instance) = voice
                .instance
                .and_then(|instance| instances.get_mut(&instance))
            {
                instance.stop(AudioTween::default());
            }
        }
    });
}

/// The sfx still sounding, by row: each instance played from a `PlaySfx`, from the moment it is
/// asked for until it has ended.
#[derive(Resource, Default)]
struct Sounding(HashMap<SfxId, Vec<Voice>>);

/// One asked-for sound: its instance once the mixer has started it, which `begun` records having
/// been seen among the loaded instances, so its later absence means it ended.
struct Voice {
    instance: Option<Handle<AudioInstance>>,
    asked: Seconds,
    begun: bool,
}

/// How long an instance may stay unheard of after it was asked for, while its clip loads, before
/// it counts as never having played.
const STARTS_WITHIN: Seconds = Seconds(2.0);

impl Sounding {
    fn settle(&mut self, instances: &Assets<AudioInstance>, now: Seconds) {
        for voices in self.0.values_mut() {
            voices.retain_mut(|voice| {
                match voice
                    .instance
                    .as_ref()
                    .and_then(|handle| instances.get(handle))
                {
                    Some(instance) => {
                        voice.begun = true;
                        instance.state() != PlaybackState::Stopped
                    }
                    None => !voice.begun && now - voice.asked < STARTS_WITHIN,
                }
            });
        }
        self.0.retain(|_, voices| !voices.is_empty());
    }

    /// Hands the mixer's instance to the oldest ask for `id` still waiting for one, else records a
    /// sound no ask preceded.
    fn begun(&mut self, id: SfxId, instance: Handle<AudioInstance>, now: Seconds) {
        let voices = self.0.entry(id).or_default();
        match voices.iter_mut().find(|voice| voice.instance.is_none()) {
            Some(asked) => asked.instance = Some(instance),
            None => voices.push(Voice {
                instance: Some(instance),
                asked: now,
                begun: false,
            }),
        }
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
    let table = content.table::<SfxDef>();
    catalog.0 = table
        .ids()
        .map(|id| {
            let def = table.row(id);
            Sound {
                handle: assets.load(def.file.path()),
                volume: def.volume,
                pitch: def.pitch,
            }
        })
        .collect();
}

fn mix(
    mut requests: MessageReader<PlaySfx>,
    listener: Res<Listener>,
    mut speakers: Speakers,
    catalog: Res<Catalog>,
    mut played: ResMut<Played>,
    mut rng: ResMut<Rng>,
) {
    let clock = speakers.now();
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
        speakers.play_sfx(id, cue.handle, tune, cue.pan);
    }
}

fn hush(mut hushes: MessageReader<HushVoice>, voice: Res<AudioChannel<VoiceTrack>>) {
    if hushes.read().next().is_some() {
        voice.stop();
    }
}

fn play_clips(mut clips: MessageReader<PlayClip>, listener: Res<Listener>, mut speakers: Speakers) {
    for clip in clips.read() {
        let Some(PositionalAudio { proximity, pan }) = heard_at(clip.place, listener.0) else {
            continue;
        };
        let tune = SfxTune {
            volume: clip.tune.volume * proximity,
            ..clip.tune
        };
        let delay = clip.due.saturating_sub(speakers.time.elapsed());
        speakers.play(clip.clip.clone(), clip.category, tune, pan, delay);
    }
}

/// The track every voice clip sounds on, so the voice can be hushed as one.
#[derive(Resource)]
struct VoiceTrack;

#[derive(SystemParam)]
struct Speakers<'w> {
    audio: Res<'w, Audio>,
    voice: Res<'w, AudioChannel<VoiceTrack>>,
    audio_mix: Res<'w, AudioMix>,
    time: Res<'w, Time>,
    sounding: ResMut<'w, Sounding>,
    sources: ResMut<'w, Assets<AudioSource>>,
}

impl Speakers<'_> {
    fn now(&self) -> Seconds {
        Seconds(self.time.elapsed_secs())
    }

    fn play_sfx(&mut self, id: SfxId, clip: Handle<AudioSource>, tune: SfxTune, pan: f32) {
        if let Some(instance) = self.play(clip, AudioCategory::Effects, tune, pan, Duration::ZERO) {
            let now = self.now();
            self.sounding.begun(id, instance, now);
        }
    }

    fn play(
        &mut self,
        clip: Handle<AudioSource>,
        category: AudioCategory,
        tune: SfxTune,
        pan: f32,
        delay: Duration,
    ) -> Option<Handle<AudioInstance>> {
        let volume = tune.volume * self.audio_mix.gain(category);
        if volume <= 0.0 {
            return None;
        }
        let clip = self.delayed(clip, delay);
        let mut command = match category {
            AudioCategory::Voice => self.voice.play(clip),
            _ => self.audio.play(clip),
        };
        command
            .with_volume(decibels(volume))
            .with_playback_rate(f64::from(tune.pitch.0))
            .with_panning(pan);
        Some(command.handle())
    }

    /// The clip as a sound the audio engine starts `delay` after it is handed over, or the clip
    /// itself when it starts at once or is not loaded yet.
    fn delayed(&mut self, clip: Handle<AudioSource>, delay: Duration) -> Handle<AudioSource> {
        if delay.is_zero() {
            return clip;
        }
        let Some(source) = self.sources.get(&clip) else {
            return clip;
        };
        let sound = source.sound.clone().start_time(StartTime::Delayed(delay));
        self.sources.add(AudioSource { sound })
    }
}

fn heard_at(place: SfxPlace, listener: Option<Pos<Meters>>) -> Option<PositionalAudio> {
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
