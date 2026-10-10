use std::io::{Cursor, ErrorKind};
use std::time::Duration;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::platform::time::Instant;
use bevy::reflect::TypePath;
use bevy_kira_audio::AudioSource;
use bevy_kira_audio::prelude::{Frame, StaticSoundData, StaticSoundSettings};
use futures_timer::Delay;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, Packet};
use symphonia::core::io::MediaSourceStream;

// Decoding a whole track at once holds the thread for hundreds of milliseconds, and on the web that
// thread also feeds the audio output and runs the frame that asked for the sound, so decoding hands
// the thread back before every slice, the first included. Waiting on a timer rather than merely
// yielding is what returns control to the browser's event loop.
const SLICE: Duration = Duration::from_millis(4);

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
        let stream = MediaSourceStream::new(Box::new(Cursor::new(encoded)), Default::default());
        let mut format = symphonia::default::get_probe()
            .format(
                &Default::default(),
                stream,
                &Default::default(),
                &Default::default(),
            )?
            .format;
        let track = format
            .default_track()
            .ok_or(Error::Unsupported("no default track"))?;
        let track_id = track.id;
        let params = &track.codec_params;
        let sample_rate = params
            .sample_rate
            .ok_or(Error::Unsupported("unknown sample rate"))?;
        let mut frames = Vec::with_capacity(params.n_frames.unwrap_or_default() as usize);
        let mut decoder = symphonia::default::get_codecs().make(params, &Default::default())?;
        let mut slice_end = Instant::now();
        loop {
            if Instant::now() >= slice_end {
                Delay::new(Duration::ZERO).await;
                slice_end = Instant::now() + SLICE;
            }
            let Some(packet) = next_packet(format.as_mut())? else {
                break;
            };
            if packet.track_id() == track_id {
                append_frames(&mut frames, decoder.decode(&packet)?)?;
            }
        }
        Ok(AudioSource {
            sound: StaticSoundData {
                sample_rate,
                frames: frames.into(),
                settings: StaticSoundSettings::default(),
                slice: None,
            },
        })
    }

    fn extensions(&self) -> &[&str] {
        &["flac", "wav"]
    }
}

fn next_packet(format: &mut dyn FormatReader) -> Result<Option<Packet>, Error> {
    match format.next_packet() {
        Ok(packet) => Ok(Some(packet)),
        Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn append_frames(frames: &mut Vec<Frame>, decoded: AudioBufferRef) -> Result<(), Error> {
    let spec = *decoded.spec();
    let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
    samples.copy_interleaved_ref(decoded);
    match spec.channels.count() {
        1 => frames.extend(
            samples
                .samples()
                .iter()
                .map(|&sample| Frame::from_mono(sample)),
        ),
        2 => frames.extend(
            samples
                .samples()
                .chunks_exact(2)
                .map(|pair| Frame::new(pair[0], pair[1])),
        ),
        _ => return Err(Error::Unsupported("more than two channels")),
    }
    Ok(())
}
