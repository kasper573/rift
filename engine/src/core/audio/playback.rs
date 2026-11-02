use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use kira::sound::PlaybackState;
use kira::sound::static_sound::StaticSoundHandle;
use kira::{Panning, StartTime, Tween};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Hearing;
use crate::core::audio::decibels;
use crate::core::audio::loader::{ClipLoader, SoundClip, SoundTrack, TrackLoader};
use crate::core::audio::mix::{AudioCategory, AudioMix};
use crate::core::audio::output::AudioOutput;
use crate::core::content::{AssetKind, AudioRef};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, FileShape, FileWriter, Nearby, PartSpec, Parts,
};
use crate::core::math::{Pos, Rng, Size};
use crate::core::units::{Meters, PlaybackRate, Seconds};

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        AudioOutput::plugin(app);
        app.init_asset::<SoundClip>()
            .init_asset::<SoundTrack>()
            .register_asset_loader(ClipLoader)
            .register_asset_loader(TrackLoader)
            .init_resource::<Listener>()
            .init_resource::<AudioMix>()
            .init_resource::<Catalog>()
            .init_resource::<Played>()
            .init_resource::<Sounding>()
            .init_resource::<VoiceClips>()
            .add_message::<PlaySfx>()
            .add_message::<PlayClip>()
            .add_message::<HushVoice>()
            .add_systems(
                Update,
                (
                    load.run_if(resource_exists_and_changed::<Content>),
                    mix.run_if(resource_exists::<Hearing>),
                    hush,
                    play_clips.run_if(resource_exists::<Hearing>),
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
    /// How long the clip plays.
    #[serde(skip)]
    pub lasts: Seconds,
    pub volume: SfxScalar,
    pub pitch: SfxScalar,
}

pub const SFX_CLIP: PartSpec = PartSpec::File {
    name: "clip",
    exts: AssetKind::Audio.extensions(),
};

impl ContentFile for SfxDef {
    const TABLE: &'static str = "sfx";
    const SHAPE: FileShape = FileShape::Bundle;

    fn fresh(_: &Nearby) -> Result<SfxDef, String> {
        Ok(SfxDef {
            file: AudioRef::default(),
            lasts: Seconds::default(),
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
            self.lasts = parts.sound_length(clip).unwrap_or_default();
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
    pub clip: Handle<SoundClip>,
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

    /// A sound at `source` as heard at `listener`, fading out `reach` away each way and panned
    /// fully aside `across` away.
    pub fn of(
        listener: Pos<Meters>,
        source: Pos<Meters>,
        reach: Size<Meters>,
        across: Meters,
    ) -> PositionalAudio {
        let offset = source - listener;
        let dx = offset.x.abs() / reach.width;
        let dy = offset.y.abs() / reach.height;
        PositionalAudio {
            proximity: (1.0 - dx.max(dy)).clamp(0.0, 1.0),
            pan: (offset.x / across.0).clamp(-1.0, 1.0),
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
            playing: None,
            asked: now,
        });
    world.write_message(request);
}

/// Whether an sfx asked for still sounds.
pub fn is_sounding(world: &mut World, id: SfxId) -> bool {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let mut sounding = world.resource_mut::<Sounding>();
    sounding.settle(now);
    sounding.0.contains_key(&id)
}

/// Stops every instance of an sfx still sounding.
pub fn stop(world: &mut World, id: SfxId) {
    let voices = world
        .resource_mut::<Sounding>()
        .0
        .remove(&id)
        .unwrap_or_default();
    for mut playing in voices.into_iter().filter_map(|voice| voice.playing) {
        playing.stop(Tween::default());
    }
}

/// The sfx still sounding, by row: each instance played from a `PlaySfx`, from the moment it is
/// asked for until it has ended.
#[derive(Resource, Default)]
struct Sounding(HashMap<SfxId, Vec<Voice>>);

/// One asked-for sound, and the sound once the mixer has started it.
struct Voice {
    playing: Option<StaticSoundHandle>,
    asked: Seconds,
}

/// How long an instance may stay unheard of after it was asked for, while its clip loads, before
/// it counts as never having played.
const STARTS_WITHIN: Seconds = Seconds(2.0);

impl Sounding {
    fn settle(&mut self, now: Seconds) {
        for voices in self.0.values_mut() {
            voices.retain(|voice| match &voice.playing {
                Some(playing) => playing.state() != PlaybackState::Stopped,
                None => now - voice.asked < STARTS_WITHIN,
            });
        }
        self.0.retain(|_, voices| !voices.is_empty());
    }

    /// Hands the mixer's sound to the oldest ask for `id` still waiting for one, else records a
    /// sound no ask preceded.
    fn begun(&mut self, id: SfxId, playing: StaticSoundHandle, now: Seconds) {
        let voices = self.0.entry(id).or_default();
        match voices.iter_mut().find(|voice| voice.playing.is_none()) {
            Some(asked) => asked.playing = Some(playing),
            None => voices.push(Voice {
                playing: Some(playing),
                asked: now,
            }),
        }
    }
}

#[derive(Resource, Default)]
struct Catalog(Vec<Sound>);

struct Sound {
    handle: Handle<SoundClip>,
    volume: SfxScalar,
    pitch: SfxScalar,
}

#[derive(Resource, Default)]
struct Played(HashMap<SfxId, Seconds>);

#[derive(Clone)]
struct Cue {
    proximity: f32,
    pan: f32,
    handle: Handle<SoundClip>,
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
    hearing: Res<Hearing>,
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
        let Some(PositionalAudio { proximity, pan }) = heard_at(req.place, listener.0, &hearing)
        else {
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
        if !ready(&mut played.0, id, clock, hearing.repeats_after) {
            continue;
        }
        let tune = SfxTune {
            pitch: PlaybackRate(cue.pitch.resolve(&mut rng)),
            volume: cue.volume.resolve(&mut rng) * cue.proximity,
        };
        speakers.play_sfx(id, &cue.handle, tune, cue.pan);
    }
}

fn hush(mut hushes: MessageReader<HushVoice>, mut voices: ResMut<VoiceClips>) {
    if hushes.read().next().is_some() {
        for mut clip in voices.0.drain(..) {
            clip.stop(Tween::default());
        }
    }
}

fn play_clips(
    mut clips: MessageReader<PlayClip>,
    listener: Res<Listener>,
    hearing: Res<Hearing>,
    mut speakers: Speakers,
) {
    for clip in clips.read() {
        let Some(PositionalAudio { proximity, pan }) = heard_at(clip.place, listener.0, &hearing)
        else {
            continue;
        };
        let tune = SfxTune {
            volume: clip.tune.volume * proximity,
            ..clip.tune
        };
        let delay = clip.due.saturating_sub(speakers.time.elapsed());
        let Some(playing) = speakers.play(&clip.clip, clip.category, tune, pan, delay) else {
            continue;
        };
        if clip.category == AudioCategory::Voice {
            speakers.voices.keep(playing);
        }
    }
}

/// The voice's clips sounding and due, so the voice can be hushed as one.
#[derive(Resource, Default)]
struct VoiceClips(Vec<StaticSoundHandle>);

impl VoiceClips {
    fn keep(&mut self, clip: StaticSoundHandle) {
        self.0.retain(|clip| clip.state() != PlaybackState::Stopped);
        self.0.push(clip);
    }
}

#[derive(SystemParam)]
struct Speakers<'w> {
    output: Option<NonSendMut<'w, AudioOutput>>,
    audio_mix: Res<'w, AudioMix>,
    time: Res<'w, Time>,
    sounding: ResMut<'w, Sounding>,
    voices: ResMut<'w, VoiceClips>,
    clips: Res<'w, Assets<SoundClip>>,
}

impl Speakers<'_> {
    fn now(&self) -> Seconds {
        Seconds(self.time.elapsed_secs())
    }

    fn play_sfx(&mut self, id: SfxId, clip: &Handle<SoundClip>, tune: SfxTune, pan: f32) {
        if let Some(playing) = self.play(clip, AudioCategory::Effects, tune, pan, Duration::ZERO) {
            let now = self.now();
            self.sounding.begun(id, playing, now);
        }
    }

    /// Starts `clip` on `category`'s track, `delay` from now; none when it would be silent, isn't
    /// loaded yet or there is no sound device.
    fn play(
        &mut self,
        clip: &Handle<SoundClip>,
        category: AudioCategory,
        tune: SfxTune,
        pan: f32,
        delay: Duration,
    ) -> Option<StaticSoundHandle> {
        if tune.volume * self.audio_mix.gain(category) <= 0.0 {
            return None;
        }
        let start = if delay.is_zero() {
            StartTime::Immediate
        } else {
            StartTime::Delayed(delay)
        };
        let sound = self
            .clips
            .get(clip)?
            .0
            .clone()
            .volume(decibels(tune.volume))
            .playback_rate(f64::from(tune.pitch.0))
            .panning(Panning(pan))
            .start_time(start);
        self.output.as_mut()?.play(category, sound)
    }
}

fn heard_at(
    place: SfxPlace,
    listener: Option<Pos<Meters>>,
    hearing: &Hearing,
) -> Option<PositionalAudio> {
    let heard = match (place, listener) {
        (SfxPlace::Interface, _) => PositionalAudio::CENTERED,
        (SfxPlace::World(at), Some(listener)) => PositionalAudio::of(
            listener,
            at,
            Size::new(hearing.wide.0, hearing.deep.0),
            hearing.wide,
        ),
        (SfxPlace::World(_), None) => return None,
    };
    heard.audible().then_some(heard)
}

fn ready(
    played: &mut HashMap<SfxId, Seconds>,
    id: SfxId,
    clock: Seconds,
    repeats_after: Seconds,
) -> bool {
    if played
        .get(&id)
        .is_some_and(|&last| clock - last < repeats_after)
    {
        return false;
    }
    played.insert(id, clock);
    true
}
