use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use bevy::prelude::*;
use bevy_kira_audio::prelude::{Audio, AudioControl, AudioInstance, AudioSource, AudioTween};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::assets::AudioRef;
use crate::core::audio::decibels;
use crate::core::audio::mix::{AudioCategory, AudioMix};
use crate::core::audio::playback::{Listener, PositionalAudio};
use crate::core::content::serial;
use crate::core::math::{Pos, Rect, Size};
use crate::core::time::Seconds;
use crate::core::units::Meters;

pub const EDGE_FALLOFF: Meters = Meters(4.0);
// The quickest the mix may swing from silence to full. Proximity drives every change, and this only
// catches the jumps, like travelling between areas, so they glide instead of cutting.
const SLEW: Seconds = Seconds(0.75);

pub struct SoundscapePlugin;

impl Plugin for SoundscapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Soundscape>()
            .init_resource::<SoundscapeMixer>()
            .add_systems(
                Update,
                (load_sources.run_if(resource_changed::<Soundscape>), mix).chain(),
            );
    }
}

#[derive(Resource, Default)]
pub struct Soundscape(pub Vec<SoundscapeZone>);

#[derive(Clone, Debug, PartialEq)]
pub struct SoundscapeZone {
    pub shape: SoundscapeShape,
    pub emit: Meters,
    pub channels: BTreeMap<SoundscapeChannel, SoundscapeLayer>,
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SoundscapeShape {
    Rect(
        #[schemars(with = "serial::RectRow")]
        #[serde(
            serialize_with = "serial::rect",
            deserialize_with = "serial::rect_from"
        )]
        Rect<Meters>,
    ),
    Ellipse(
        #[schemars(with = "serial::RectRow")]
        #[serde(
            serialize_with = "serial::rect",
            deserialize_with = "serial::rect_from"
        )]
        Rect<Meters>,
    ),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundscapeChannel(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct SoundscapeLayer {
    pub src: AudioRef,
    pub volume: f32,
    pub pan: f32,
    pub category: AudioCategory,
}

#[derive(Resource, Default)]
pub struct SoundscapeMixer {
    clock_start: Duration,
    sources: HashMap<AudioRef, Handle<AudioSource>>,
    voices: Vec<SoundscapeVoice>,
    refused: Option<Vec<WantedLayer>>,
}

pub struct SoundscapeVoice {
    pub channel: SoundscapeChannel,
    pub category: AudioCategory,
    pub src: AudioRef,
    instance: Handle<AudioInstance>,
    share: f32,
    volume: f32,
    pan: f32,
    applied: Option<(f32, f32)>,
}

impl SoundscapeZone {
    pub fn reach(&self) -> Meters {
        EDGE_FALLOFF + self.emit
    }

    pub fn heard_from(&self, at: Pos<Meters>) -> Option<PositionalAudio> {
        let heard = PositionalAudio::of(at, self.shape.nearest(at), Size::splat(self.reach().0));
        heard.audible().then_some(heard)
    }
}

impl SoundscapeShape {
    pub fn bounds(self) -> Rect<Meters> {
        match self {
            SoundscapeShape::Rect(bounds) | SoundscapeShape::Ellipse(bounds) => bounds,
        }
    }

    pub fn nearest(self, at: Pos<Meters>) -> Pos<Meters> {
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
        instance.set_decibels(decibels(mixed.0), tween);
        instance.set_panning(mixed.1, tween);
        self.applied = Some(mixed);
    }
}

#[derive(Clone, PartialEq)]
struct WantedLayer {
    channel: SoundscapeChannel,
    category: AudioCategory,
    src: AudioRef,
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
    assets: Res<AssetServer>,
    mut mixer: ResMut<SoundscapeMixer>,
) {
    mixer.clock_start = time.elapsed();
    mixer.sources = soundscape
        .0
        .iter()
        .flat_map(|zone| zone.channels.values())
        .map(|layer| (layer.src.clone(), assets.load(layer.src.path())))
        .collect();
}

#[expect(clippy::too_many_arguments)]
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
    let elapsed = Seconds(time.delta_secs());
    if mixer
        .refused
        .as_ref()
        .is_some_and(|refused| *refused != wanted)
    {
        mixer.refused = None;
    }
    mixer.voices.retain_mut(|voice| {
        let playing = wanted.iter().find(|layer| voice.plays(layer));
        voice.approach(playing, elapsed);
        let Some(mut instance) = instances.get_mut(&voice.instance) else {
            warn!("soundscape voice {} failed to start", voice.src.as_str());
            mixer.refused = Some(wanted.clone());
            return false;
        };
        if playing.is_none() && voice.share <= 0.0 {
            instance.stop(AudioTween::default());
            return false;
        }
        voice.apply(&mut instance, audio_mix.gain(voice.category), elapsed);
        true
    });
    if mixer.refused.is_some() {
        return;
    }
    for layer in &wanted {
        if mixer.voices.iter().any(|voice| voice.plays(layer)) {
            continue;
        }
        let Some(source) = mixer.sources.get(&layer.src) else {
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
            category: layer.category,
            src: layer.src.clone(),
            instance,
            share: 0.0,
            volume: layer.volume,
            pan: layer.pan,
            applied: None,
        });
    }
}

fn wanted_layers(soundscape: &Soundscape, at: Pos<Meters>) -> Vec<WantedLayer> {
    let mut wanted: Vec<WantedLayer> = Vec::new();
    for zone in &soundscape.0 {
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
                    category: layer.category,
                    src: layer.src.clone(),
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

fn unit_circle(bounds: Rect<Meters>, at: Pos<Meters>) -> euclid::Vector2D<f32, Meters> {
    (at - bounds.center()).component_div(bounds.size.to_vector() / 2.0)
}

fn toward(from: f32, to: f32, step: f32) -> f32 {
    from + (to - from).clamp(-step, step)
}
