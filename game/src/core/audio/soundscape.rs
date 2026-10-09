use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioInstance, AudioSource, AudioTween};
use serde::Deserialize;

use crate::core::assets::AssetService;
use crate::core::audio::decibels;
use crate::core::audio::decode::DecodingTrack;
use crate::core::audio::mix::{AudioCategory, AudioMix};
use crate::core::audio::playback::{Listener, PositionalAudio};
use crate::core::math::{Pos, Rect, Size};
use crate::core::tiling::Tiles;
use crate::core::time::Seconds;

const EDGE_FALLOFF: Tiles = Tiles(4.0);
// The quickest the mix may swing from silence to full. Proximity drives every change, and this only
// catches the jumps, like travelling between areas, so they glide instead of cutting.
const SLEW: Seconds = Seconds(0.75);
// Decoding a whole track at once stalls the frame for long enough to starve audio output that is
// fed from the same thread (as on the web), so tracks decode a little each frame instead.
const DECODE_BUDGET: Duration = Duration::from_millis(4);

pub struct SoundscapePlugin;

impl Plugin for SoundscapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Soundscape>()
            .init_resource::<SoundscapeMixer>()
            .add_systems(
                Update,
                (
                    load_sources.run_if(resource_changed::<Soundscape>),
                    decode_sources,
                    mix,
                )
                    .chain(),
            );
    }
}

#[derive(Resource, Default)]
pub struct Soundscape(pub &'static [SoundscapeZone]);

#[derive(Clone, Debug, PartialEq)]
pub struct SoundscapeZone {
    pub name: String,
    pub shape: SoundscapeShape,
    pub emit: Tiles,
    pub channels: BTreeMap<SoundscapeChannel, SoundscapeLayer>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundscapeShape {
    Rect(Rect<Tiles>),
    Ellipse(Rect<Tiles>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundscapeChannel(pub u32);

/// Maps write a layer as comma separated values in field order, and may leave off those after `src`.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct SoundscapeLayer {
    pub src: String,
    #[serde(default = "full_volume")]
    pub volume: f32,
    #[serde(default)]
    pub pan: f32,
}

#[derive(Resource, Default)]
pub struct SoundscapeMixer {
    clock_start: Duration,
    sources: HashMap<&'static str, SoundscapeSource>,
    voices: Vec<SoundscapeVoice>,
}

pub struct SoundscapeVoice {
    pub channel: SoundscapeChannel,
    pub src: &'static str,
    instance: Handle<AudioInstance>,
    share: f32,
    volume: f32,
    pan: f32,
    applied: Option<(f32, f32)>,
}

impl SoundscapeChannel {
    pub const MUSIC: SoundscapeChannel = SoundscapeChannel(1);

    pub fn category(self) -> AudioCategory {
        match self {
            SoundscapeChannel::MUSIC => AudioCategory::Music,
            _ => AudioCategory::Ambience,
        }
    }
}

impl SoundscapeZone {
    pub fn reach(&self) -> Tiles {
        EDGE_FALLOFF + self.emit
    }

    pub fn heard_from(&self, at: Pos<Tiles>) -> Option<PositionalAudio> {
        let heard = PositionalAudio::of(at, self.shape.nearest(at), Size::splat(self.reach().0));
        heard.audible().then_some(heard)
    }
}

impl SoundscapeShape {
    pub fn bounds(self) -> Rect<Tiles> {
        match self {
            SoundscapeShape::Rect(bounds) | SoundscapeShape::Ellipse(bounds) => bounds,
        }
    }

    fn nearest(self, at: Pos<Tiles>) -> Pos<Tiles> {
        match self {
            SoundscapeShape::Rect(bounds) => at.clamp(bounds.min(), bounds.max()),
            SoundscapeShape::Ellipse(bounds) => {
                let toward = unit_circle(bounds, at);
                let edge = toward / toward.length().max(1.0);
                bounds.center() + edge.component_mul(bounds.size.to_vector() / 2.0)
            }
        }
    }
}

impl FromStr for SoundscapeLayer {
    type Err = String;

    fn from_str(values: &str) -> Result<SoundscapeLayer, String> {
        let layer: SoundscapeLayer = csv::ReaderBuilder::new()
            .has_headers(false)
            .trim(csv::Trim::All)
            .from_reader(values.as_bytes())
            .deserialize()
            .next()
            .ok_or("has no values")?
            .map_err(|error| error.to_string())?;
        if layer.volume < 0.0 {
            return Err(format!("volume {} is below 0", layer.volume));
        }
        if !(-1.0..=1.0).contains(&layer.pan) {
            return Err(format!("pan {} is outside -1..1", layer.pan));
        }
        Ok(layer)
    }
}

impl SoundscapeMixer {
    pub fn voices(&self) -> &[SoundscapeVoice] {
        &self.voices
    }
}

impl SoundscapeVoice {
    pub fn heard(&self) -> f32 {
        self.applied.map_or(0.0, |(amplitude, _)| amplitude)
    }

    fn level(&self) -> f32 {
        // Distinct tracks share a channel by power, so a crossfade between them holds its loudness.
        self.share.sqrt() * self.volume
    }

    fn plays(&self, layer: &WantedLayer) -> bool {
        self.channel == layer.channel && self.src == layer.src
    }

    fn approach(&mut self, wanted: Option<&WantedLayer>, elapsed: Seconds) {
        let step = elapsed.ratio(SLEW);
        self.share = toward(self.share, wanted.map_or(0.0, |wanted| wanted.share), step);
        if let Some(wanted) = wanted {
            self.volume = toward(self.volume, wanted.volume, step);
            self.pan = toward(self.pan, wanted.pan, 2.0 * step);
        }
    }

    fn apply(&mut self, instance: &mut AudioInstance, gain: f32, elapsed: Seconds) {
        let mixed = (self.level() * gain, self.pan);
        if self.applied == Some(mixed) {
            return;
        }
        let tween = AudioTween::linear(elapsed.into());
        instance.set_decibels(decibels(mixed.0), tween.clone());
        instance.set_panning(mixed.1, tween);
        self.applied = Some(mixed);
    }
}

enum SoundscapeSource {
    Decoding(DecodingTrack),
    Decoded(Handle<AudioSource>),
}

struct WantedLayer {
    channel: SoundscapeChannel,
    src: &'static str,
    share: f32,
    volume: f32,
    pan: f32,
}

impl WantedLayer {
    fn blend(&mut self, share: f32, layer: &SoundscapeLayer, pan: f32) {
        let weight = share / (self.share + share);
        self.volume += (layer.volume - self.volume) * weight;
        self.pan += (pan - self.pan) * weight;
        self.share += share;
    }
}

fn load_sources(
    soundscape: Res<Soundscape>,
    time: Res<Time<Real>>,
    service: Res<AssetService>,
    mut mixer: ResMut<SoundscapeMixer>,
) {
    mixer.clock_start = time.elapsed();
    let srcs: HashSet<&'static str> = soundscape
        .0
        .iter()
        .flat_map(|zone| zone.channels.values())
        .map(|layer| layer.src.as_str())
        .collect();
    mixer.sources.retain(|src, _| srcs.contains(src));
    for src in srcs {
        let Entry::Vacant(slot) = mixer.sources.entry(src) else {
            continue;
        };
        match DecodingTrack::open(&service, Path::new(src)) {
            Ok(track) => {
                slot.insert(SoundscapeSource::Decoding(track));
            }
            Err(error) => warn!("soundscape track {src}: {error}"),
        }
    }
}

fn decode_sources(mut mixer: ResMut<SoundscapeMixer>, mut sources: ResMut<Assets<AudioSource>>) {
    let deadline = Instant::now() + DECODE_BUDGET;
    mixer.sources.retain(|src, source| {
        let SoundscapeSource::Decoding(track) = source else {
            return true;
        };
        match track.decode_until(deadline) {
            Ok(None) => true,
            Ok(Some(decoded)) => {
                *source = SoundscapeSource::Decoded(sources.add(decoded));
                true
            }
            Err(error) => {
                warn!("soundscape track {src}: {error}");
                false
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn mix(
    soundscape: Res<Soundscape>,
    listener: Res<Listener>,
    time: Res<Time<Real>>,
    audio_mix: Res<AudioMix>,
    audio: Res<Audio>,
    sources: Res<Assets<AudioSource>>,
    mut instances: ResMut<Assets<AudioInstance>>,
    mut mixer: ResMut<SoundscapeMixer>,
) {
    let mixer = &mut *mixer;
    let wanted = listener
        .0
        .map_or_else(Vec::new, |at| wanted_layers(&soundscape, at));
    let clock = time.elapsed().saturating_sub(mixer.clock_start);
    for layer in &wanted {
        if mixer.voices.iter().any(|voice| voice.plays(layer)) {
            continue;
        }
        let Some(SoundscapeSource::Decoded(source)) = mixer.sources.get(layer.src) else {
            continue;
        };
        let Some(clip) = sources.get(source) else {
            continue;
        };
        // Unheard tracks are virtual: one starts where it would be had it played all along.
        let instance = audio
            .play(source.clone())
            .looped()
            .start_from(clock.as_secs_f64() % clip.sound.duration().as_secs_f64())
            .with_volume(decibels(0.0))
            .with_panning(layer.pan)
            .handle();
        mixer.voices.push(SoundscapeVoice {
            channel: layer.channel,
            src: layer.src,
            instance,
            share: 0.0,
            volume: layer.volume,
            pan: layer.pan,
            applied: None,
        });
    }
    let elapsed = Seconds(time.delta_secs());
    mixer.voices.retain_mut(|voice| {
        let wanted = wanted.iter().find(|layer| voice.plays(layer));
        voice.approach(wanted, elapsed);
        let Some(mut instance) = instances.get_mut(&voice.instance) else {
            return true;
        };
        if wanted.is_none() && voice.share <= 0.0 {
            instance.stop(AudioTween::default());
            return false;
        }
        voice.apply(
            &mut instance,
            audio_mix.gain(voice.channel.category()),
            elapsed,
        );
        true
    });
}

fn wanted_layers(soundscape: &Soundscape, at: Pos<Tiles>) -> Vec<WantedLayer> {
    let mut wanted: Vec<WantedLayer> = Vec::new();
    for zone in soundscape.0 {
        let Some(heard) = zone.heard_from(at) else {
            continue;
        };
        let power = heard.proximity * heard.proximity;
        for (&channel, layer) in &zone.channels {
            let pan = (layer.pan + heard.pan).clamp(-1.0, 1.0);
            match wanted
                .iter_mut()
                .find(|wanted| wanted.channel == channel && wanted.src == layer.src)
            {
                Some(wanted) => wanted.blend(power, layer, pan),
                None => wanted.push(WantedLayer {
                    channel,
                    src: &layer.src,
                    share: power,
                    volume: layer.volume,
                    pan,
                }),
            }
        }
    }
    // A channel carries one track at full power, so tracks heard on it at once split that power.
    let mut channel_power: BTreeMap<SoundscapeChannel, f32> = BTreeMap::new();
    for layer in &wanted {
        *channel_power.entry(layer.channel).or_default() += layer.share;
    }
    for layer in &mut wanted {
        layer.share /= channel_power[&layer.channel].max(1.0);
    }
    wanted
}

fn unit_circle(bounds: Rect<Tiles>, at: Pos<Tiles>) -> euclid::Vector2D<f32, Tiles> {
    (at - bounds.center()).component_div(bounds.size.to_vector() / 2.0)
}

fn toward(from: f32, to: f32, step: f32) -> f32 {
    from + (to - from).clamp(-step, step)
}

fn full_volume() -> f32 {
    1.0
}
