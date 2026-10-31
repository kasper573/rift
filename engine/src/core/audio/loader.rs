use std::io::Cursor;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use bevy_kira_audio::AudioSource;
use bevy_kira_audio::prelude::{Frame, StaticSoundData, StaticSoundSettings};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error;
use symphonia::core::formats::TrackType;
use symphonia::core::io::MediaSourceStream;

use crate::core::assets::AssetKind;

pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl DecodedAudio {
    pub fn into_sound(self) -> Result<StaticSoundData, Error> {
        let frames: Vec<Frame> = match self.channels {
            1 => self
                .samples
                .iter()
                .map(|&sample| Frame::from_mono(sample))
                .collect(),
            2 => self
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[left, right]| Frame::new(left, right))
                .collect(),
            _ => return Err(Error::Unsupported("more than two channels")),
        };
        Ok(StaticSoundData {
            sample_rate: self.sample_rate,
            frames: frames.into(),
            settings: StaticSoundSettings::default(),
            slice: None,
        })
    }
}

pub fn decode(encoded: Vec<u8>) -> Result<DecodedAudio, Error> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
    let mut format = symphonia::default::get_probe().probe(
        &Default::default(),
        stream,
        Default::default(),
        Default::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(Error::Unsupported("no audio track"))?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err(Error::Unsupported("unknown audio codec"));
    };
    let sample_rate = params
        .sample_rate
        .ok_or(Error::Unsupported("unknown sample rate"))?;
    let mut channels = params
        .channels
        .as_ref()
        .map_or(0, |channels| channels.count());
    let codec = symphonia::default::get_codecs()
        .get_audio_decoder(params.codec)
        .ok_or(Error::Unsupported("no decoder for the audio codec"))?;
    let mut decoder = (codec.factory)(&params, &Default::default())?;
    let mut samples = Vec::with_capacity(track.num_frames.unwrap_or_default() as usize * channels);
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        let audio = decoder.decode(&packet)?;
        channels = audio.spec().channels().count();
        let start = samples.len();
        samples.resize(start + audio.frames() * channels, 0.0);
        audio.copy_to_slice_interleaved(&mut samples[start..]);
    }
    Ok(DecodedAudio {
        sample_rate,
        channels,
        samples,
    })
}

/// Reads a sound whole.
#[derive(TypePath)]
pub struct AudioLoader;

impl AssetLoader for AudioLoader {
    type Asset = AudioSource;
    type Settings = ();
    type Error = Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<AudioSource, Error> {
        let mut encoded = Vec::new();
        reader.read_to_end(&mut encoded).await?;
        Ok(AudioSource {
            sound: decode(encoded)?.into_sound()?,
        })
    }

    fn extensions(&self) -> &[&str] {
        AssetKind::Audio.extensions()
    }
}
