use std::collections::HashMap;
use ui::egui;

use engine::core::content::sound;

use std::sync::mpsc::TryRecvError;

use crate::inbox::ChannelInbox;
use crate::project::ProjectFiles;

pub const BINS: usize = 160;

#[derive(Clone)]
pub struct Waveform {
    pub bins: Vec<f32>,
    pub ms: u32,
}

/// The waveforms of sounds, each measured once per version of its file, away from the frame that
/// asks: decoding a sound takes far longer than a frame.
#[derive(Default)]
pub struct WaveformCache {
    waves: HashMap<String, Measured>,
}

/// What is known of a sound's waveform.
#[derive(Clone)]
pub enum WaveState {
    Measuring,
    Ready(Waveform),
    /// The file is missing or doesn't decode.
    Unreadable,
}

impl WaveState {
    pub fn ready(self) -> Option<Waveform> {
        match self {
            WaveState::Ready(wave) => Some(wave),
            WaveState::Measuring | WaveState::Unreadable => None,
        }
    }
}

struct Measured {
    hash: String,
    state: Measuring,
}

enum Measuring {
    Pending(ChannelInbox<Option<Waveform>>),
    Done(Option<Waveform>),
}

impl WaveformCache {
    /// The waveform of the sound at `path`, once it is measured; until then the measuring runs
    /// on, and `ctx` is asked to draw again when it ends.
    pub fn wave(
        &mut self,
        ctx: &egui::Context,
        files: &ProjectFiles,
        path: &str,
    ) -> Option<Waveform> {
        self.state(ctx, files, path).ready()
    }

    pub fn state(&mut self, ctx: &egui::Context, files: &ProjectFiles, path: &str) -> WaveState {
        let (Some(hash), Some(bytes)) = (
            files.stamp_of(path).map(|stamp| stamp.to_string()),
            files.get(path),
        ) else {
            return WaveState::Unreadable;
        };
        let measured = match self.waves.get_mut(path) {
            Some(measured) if measured.hash == hash => measured,
            _ => {
                let (sender, receiver) = std::sync::mpsc::channel();
                let bytes = bytes.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = sender.send(analyse(&bytes));
                    ctx.request_repaint();
                });
                self.waves.insert(
                    path.to_owned(),
                    Measured {
                        hash: hash.to_owned(),
                        state: Measuring::Pending(ChannelInbox::new(receiver)),
                    },
                );
                self.waves.get_mut(path).expect("just measured")
            }
        };
        if let Measuring::Pending(inbox) = &measured.state {
            match inbox.try_recv() {
                Ok(wave) => measured.state = Measuring::Done(wave),
                Err(TryRecvError::Empty) => return WaveState::Measuring,
                Err(TryRecvError::Disconnected) => measured.state = Measuring::Done(None),
            }
        }
        match &measured.state {
            Measuring::Done(Some(wave)) => WaveState::Ready(wave.clone()),
            Measuring::Done(None) => WaveState::Unreadable,
            Measuring::Pending(_) => WaveState::Measuring,
        }
    }
}

fn analyse(bytes: &[u8]) -> Option<Waveform> {
    let decoded = sound::decode(bytes).ok()?;
    let frames = decoded.frames.len();
    let ms = (frames as f64 * 1000.0 / decoded.sample_rate.max(1) as f64).round() as u32;
    let mut bins = vec![0.0f32; BINS];
    for (index, frame) in decoded.frames.iter().enumerate() {
        let bin = (index * BINS / frames.max(1)).min(BINS - 1);
        bins[bin] = bins[bin].max(frame.left.abs()).max(frame.right.abs());
    }
    let peak = bins.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    for bin in &mut bins {
        *bin /= peak;
    }
    Some(Waveform { bins, ms })
}
