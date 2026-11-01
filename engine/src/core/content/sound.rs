use std::io::Cursor;

use bevy_kira_audio::prelude::{Frame, StaticSoundData, StaticSoundSettings};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, TrackType};

use crate::core::units::Seconds;
use symphonia::core::io::MediaSourceStream;

pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl DecodedAudio {
    pub fn length(&self) -> Seconds {
        let frames = self.samples.len() / self.channels.max(1);
        Seconds(frames as f32 / self.sample_rate as f32)
    }

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

/// How long the sound of `encoded` plays, read from its header alone.
pub fn length(encoded: &[u8]) -> Result<Seconds, Error> {
    let format = open(encoded.to_vec())?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(Error::Unsupported("no audio track"))?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(Error::Unsupported("unknown audio codec"));
    };
    let rate = params
        .sample_rate
        .ok_or(Error::Unsupported("unknown sample rate"))?;
    let frames = track
        .num_frames
        .ok_or(Error::Unsupported("unknown length"))?;
    Ok(Seconds(frames as f32 / rate as f32))
}

pub fn decode(encoded: Vec<u8>) -> Result<DecodedAudio, Error> {
    let mut format = open(encoded)?;
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

fn open(encoded: Vec<u8>) -> Result<Box<dyn FormatReader>, Error> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
    symphonia::default::get_probe().probe(
        &Default::default(),
        stream,
        Default::default(),
        Default::default(),
    )
}
