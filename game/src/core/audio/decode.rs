use std::io::{Cursor, ErrorKind, Read};
use std::path::Path;

use bevy::platform::time::Instant;
use bevy_kira_audio::AudioSource;
use bevy_kira_audio::prelude::{Frame, StaticSoundData, StaticSoundSettings};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;

use crate::core::assets::AssetService;

pub struct DecodingTrack {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    frames: Vec<Frame>,
}

impl DecodingTrack {
    pub fn open(service: &AssetService, path: &Path) -> Result<DecodingTrack, Error> {
        let mut encoded = Vec::new();
        service.open(path)?.read_to_end(&mut encoded)?;
        let stream = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
        let reader = symphonia::default::get_probe()
            .format(
                &Default::default(),
                stream,
                &Default::default(),
                &Default::default(),
            )?
            .format;
        let track = reader
            .default_track()
            .ok_or(Error::Unsupported("no default track"))?;
        let params = &track.codec_params;
        Ok(DecodingTrack {
            decoder: symphonia::default::get_codecs().make(params, &Default::default())?,
            track_id: track.id,
            sample_rate: params
                .sample_rate
                .ok_or(Error::Unsupported("unknown sample rate"))?,
            frames: Vec::with_capacity(params.n_frames.unwrap_or_default() as usize),
            reader,
        })
    }

    pub fn decode_until(&mut self, deadline: Instant) -> Result<Option<AudioSource>, Error> {
        while Instant::now() < deadline {
            let packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    return Ok(Some(self.finish()));
                }
                Err(error) => return Err(error),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = self.decoder.decode(&packet)?;
            let spec = *decoded.spec();
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            samples.copy_interleaved_ref(decoded);
            match spec.channels.count() {
                1 => self.frames.extend(
                    samples
                        .samples()
                        .iter()
                        .map(|&sample| Frame::from_mono(sample)),
                ),
                2 => self.frames.extend(
                    samples
                        .samples()
                        .chunks_exact(2)
                        .map(|pair| Frame::new(pair[0], pair[1])),
                ),
                _ => return Err(Error::Unsupported("more than two channels")),
            }
        }
        Ok(None)
    }

    fn finish(&mut self) -> AudioSource {
        AudioSource {
            sound: StaticSoundData {
                sample_rate: self.sample_rate,
                frames: std::mem::take(&mut self.frames).into(),
                settings: StaticSoundSettings::default(),
                slice: None,
            },
        }
    }
}
