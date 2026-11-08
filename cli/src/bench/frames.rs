//! What the client and editor benches measure of the frames they watch, at each of the video
//! settings they are watched at, against the criteria every scenario must meet: a frame rate no
//! scenario's mean may fall below, a share of frames that may come slower than that, a stall no
//! frame may reach, and nothing the view takes in left missing.

use editor::app::DrawnPass;
use engine::core::units::{Bytes, Millis};
use engine::systems::probe::{FrameSample, ProbeSlowFrame};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::render::VideoPreset;

impl VideoPreset {
    /// The frame rate no scenario's mean may fall below at these settings.
    pub fn floor(self) -> Fps {
        match self {
            VideoPreset::Highest => Fps(60.0),
            VideoPreset::Lowest => Fps(90.0),
        }
    }
}

/// Frames a second.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Fps(pub f64);

impl Fps {
    pub fn period(self) -> Millis {
        Millis((1000.0 / self.0.max(f64::EPSILON)) as f32)
    }

    fn of(time: Millis) -> Fps {
        Fps(1000.0 / f64::from(time.0.max(f32::EPSILON)))
    }
}

/// How much a scenario asks of the machine, and so how far above the floor it is expected to
/// stay: a busy scene has only the floor to meet, a typical and a light one are expected to
/// leave headroom over it, and are warned about when they don't.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Busy,
    Typical,
    Light,
}

impl Class {
    pub fn headroom(self, video: VideoPreset) -> Option<Fps> {
        match (self, video) {
            (Class::Busy, _) => None,
            (Class::Typical, VideoPreset::Highest) => Some(Fps(100.0)),
            (Class::Typical, VideoPreset::Lowest) => Some(Fps(200.0)),
            (Class::Light, VideoPreset::Highest) => Some(Fps(150.0)),
            (Class::Light, VideoPreset::Lowest) => Some(Fps(200.0)),
        }
    }
}

/// What a scenario's frames must meet: a scenario's own may only be stricter than the global
/// ones of its video settings, but for an opening's, drawn while its view still fills.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct Criteria {
    /// The frame rate the mean over the measured window must reach.
    pub floor: Fps,
    /// The share of frames that may take longer than the floor's period.
    pub slow_share: f64,
    /// How long no frame may take.
    pub stall: Millis,
    /// How many frames may show anything the view takes in missing.
    pub missing_frames: u32,
}

impl Criteria {
    pub fn global(video: VideoPreset) -> Criteria {
        Criteria {
            floor: video.floor(),
            slow_share: 0.01,
            stall: Millis(48.0),
            missing_frames: 0,
        }
    }

    /// The global criteria, but for what the view takes in, which an opening draws while it still
    /// arrives.
    pub fn opening(video: VideoPreset) -> Criteria {
        Criteria {
            missing_frames: u32::MAX,
            ..Criteria::global(video)
        }
    }

    /// These criteria, or `own` where it is stricter.
    pub fn stricter(self, own: Criteria) -> Criteria {
        Criteria {
            floor: Fps(self.floor.0.max(own.floor.0)),
            slow_share: self.slow_share.min(own.slow_share),
            stall: Millis(self.stall.0.min(own.stall.0)),
            missing_frames: self.missing_frames.min(own.missing_frames),
        }
    }

    /// Everything `stats` fall short of, each said in a line.
    pub fn failures(&self, stats: &FrameStats) -> Vec<String> {
        let mut broken = Vec::new();
        if stats.mean.0 < self.floor.0 {
            broken.push(format!(
                "mean {:.0} fps under the floor of {:.0}",
                stats.mean.0, self.floor.0
            ));
        }
        if stats.slow_share > self.slow_share {
            broken.push(format!(
                "{:.2}% of frames slower than {:.1} ms (at most {:.2}%)",
                stats.slow_share * 100.0,
                self.floor.period().0,
                self.slow_share * 100.0
            ));
        }
        if stats.stalls > 0 {
            let waited_on = stats
                .worst_frame
                .as_ref()
                .and_then(|frame| frame.systems.first())
                .map(|system| {
                    format!(
                        ", its longest system {} ({:.1} ms)",
                        system.name, system.ms.0
                    )
                })
                .unwrap_or_default();
            broken.push(format!(
                "{} frames stalled past {:.0} ms, the worst {:.1} ms{waited_on}",
                stats.stalls, self.stall.0, stats.worst.0
            ));
        }
        if stats.missing_frames > self.missing_frames {
            broken.push(format!(
                "{} frames with the view's scenery missing (at most {})",
                stats.missing_frames, self.missing_frames
            ));
        }
        broken
    }
}

/// The frames of a scenario's measured window, as the probe reported them.
#[derive(Clone, Debug, Default)]
pub struct Window {
    samples: Vec<FrameSample>,
    slow: Vec<ProbeSlowFrame>,
    /// What the editor drew over each frame, by frame, where the editor logged it.
    passes: Vec<(u32, DrawnPass)>,
}

impl Window {
    pub fn extend(&mut self, samples: impl IntoIterator<Item = FrameSample>) {
        self.samples.extend(samples);
    }

    /// The slow frames the probe kept with their passes and the systems they waited on.
    pub fn note_slow(&mut self, slow: &[ProbeSlowFrame]) {
        for frame in slow {
            if !self.slow.iter().any(|kept| kept.frame == frame.frame) {
                self.slow.push(frame.clone());
            }
        }
    }

    /// The editor's passes over its frames, as its frame log tells them.
    pub fn note_passes(&mut self, passes: impl IntoIterator<Item = (u32, DrawnPass)>) {
        self.passes.extend(passes);
    }

    pub fn first_frame(&self) -> Option<u32> {
        self.samples.first().map(|sample| sample.frame)
    }

    /// The window without its first `frames` frames, which warm the scene up.
    pub fn after_warm_up(&self, frames: u32) -> Window {
        let Some(first) = self.first_frame() else {
            return Window::default();
        };
        let kept = self
            .samples
            .iter()
            .filter(|sample| sample.frame >= first + frames)
            .cloned()
            .collect();
        Window {
            samples: kept,
            slow: self.slow.clone(),
            passes: self.passes.clone(),
        }
    }

    fn interface(&self, first: u32, last: u32) -> Option<InterfaceTimes> {
        let passes: Vec<&DrawnPass> = self
            .passes
            .iter()
            .filter(|(frame, _)| (first..=last).contains(frame))
            .map(|(_, pass)| pass)
            .collect();
        let overlay_worst = passes
            .iter()
            .map(|pass| pass.overlay_ms.0)
            .reduce(f32::max)?;
        Some(InterfaceTimes {
            ui: median(passes.iter().map(|pass| pass.ui_ms))?,
            tree: median(passes.iter().map(|pass| pass.tree_ms))?,
            overlay: median(passes.iter().map(|pass| pass.overlay_ms))?,
            overlay_worst: Millis(overlay_worst),
        })
    }

    /// The window's frames measured against `criteria`, or nothing of an empty window.
    pub fn stats(&self, criteria: &Criteria) -> Option<FrameStats> {
        if self.samples.is_empty() {
            return None;
        }
        let mut times: Vec<f32> = self.samples.iter().map(|sample| sample.dt_ms.0).collect();
        times.sort_by(f32::total_cmp);
        let count = times.len();
        let lasted: f32 = times.iter().sum();
        let at = |share: f64| times[((count as f64 * share) as usize).min(count - 1)];
        let low_count = (count / 100).max(1);
        let low_mean = times[count - low_count..].iter().sum::<f32>() / low_count as f32;
        let period = criteria.floor.period();
        let slow = times.iter().filter(|&&time| time > period.0).count();
        let stalls = times
            .iter()
            .filter(|&&time| time > criteria.stall.0)
            .count();
        let first = self.samples.first().map_or(0, |sample| sample.frame);
        let last = self.samples.last().map_or(0, |sample| sample.frame);
        let mut stalled: Vec<ProbeSlowFrame> = self
            .slow
            .iter()
            .filter(|frame| frame.frame >= first && frame.frame <= last)
            .filter(|frame| frame.dt_ms.0 > criteria.stall.0)
            .cloned()
            .collect();
        stalled.sort_by_key(|frame| frame.frame);
        let worst = self
            .slow
            .iter()
            .filter(|frame| frame.frame >= first && frame.frame <= last)
            .max_by(|a, b| a.dt_ms.0.total_cmp(&b.dt_ms.0))
            .cloned();
        let trace = self
            .samples
            .iter()
            .map(|sample| FrameTrace {
                frame: sample.frame,
                ms: sample.dt_ms,
                cpu_main: sample.cpu_main_ms,
                cpu_render: sample.cpu_render_ms,
                gpu: sample.gpu_ms,
                props_drawn: sample.props_drawn,
            })
            .collect();
        Some(FrameStats {
            trace,
            frames: count as u32,
            seconds: f64::from(lasted) / 1000.0,
            mean: Fps(count as f64 / f64::from(lasted.max(f32::EPSILON)) * 1000.0),
            p50: Millis(at(0.5)),
            p95: Millis(at(0.95)),
            p99: Millis(at(0.99)),
            p999: Millis(at(0.999)),
            low: Fps::of(Millis(low_mean)),
            worst: Millis(times[count - 1]),
            slow_share: slow as f64 / count as f64,
            stalls: stalls as u32,
            missing_frames: self
                .samples
                .iter()
                .filter(|sample| sample.missing > 0)
                .count() as u32,
            stale_frames: self
                .samples
                .iter()
                .filter(|sample| sample.stale > 0)
                .count() as u32,
            cpu_main: median(self.samples.iter().map(|sample| sample.cpu_main_ms)),
            cpu_render: median(
                self.samples
                    .iter()
                    .filter_map(|sample| sample.cpu_render_ms),
            ),
            gpu: median(self.samples.iter().filter_map(|sample| sample.gpu_ms)),
            draws: median_u32(self.samples.iter().map(|sample| sample.draws)),
            instances: median_u32(self.samples.iter().map(|sample| sample.instances)),
            props_drawn: median_u32(self.samples.iter().filter_map(|sample| sample.props_drawn)),
            upload_bytes: median_u64(self.samples.iter().map(|sample| sample.upload_bytes.0))
                .map(Bytes),
            vfx_writes: median_u32(self.samples.iter().map(|sample| sample.vfx_writes)),
            posed: median_u32(self.samples.iter().map(|sample| sample.posed)),
            plans: Some(self.samples.iter().map(|sample| sample.plans).sum()),
            interface: self.interface(first, last),
            worst_frame: worst,
            stalled_frames: stalled,
        })
    }
}

/// What a window of frames measured, as the report reads it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FrameStats {
    pub frames: u32,
    pub seconds: f64,
    pub mean: Fps,
    pub p50: Millis,
    pub p95: Millis,
    pub p99: Millis,
    pub p999: Millis,
    /// The mean frame rate of the slowest hundredth of the frames.
    pub low: Fps,
    pub worst: Millis,
    /// The share of frames slower than the floor's period.
    pub slow_share: f64,
    /// Frames slower than the stall the criteria allow.
    pub stalls: u32,
    /// Frames that showed anything the view takes in missing.
    pub missing_frames: u32,
    /// Frames that showed anything the view takes in as it stood before an edit.
    pub stale_frames: u32,
    pub cpu_main: Option<Millis>,
    pub cpu_render: Option<Millis>,
    pub gpu: Option<Millis>,
    pub draws: Option<u32>,
    pub instances: Option<u32>,
    /// The prop instances the GPU drew a frame, where its views culled any.
    pub props_drawn: Option<u32>,
    /// What a frame commonly sent the GPU, and how many vfx materials it painted anew and bodies it
    /// posed: the work it did for what changed on screen.
    pub upload_bytes: Option<Bytes>,
    pub vfx_writes: Option<u32>,
    pub posed: Option<u32>,
    /// How many times the scenery planned anew over the window, where the report counted it.
    pub plans: Option<u32>,
    /// How long the editor took over its interface on the window's frames, where it told.
    pub interface: Option<InterfaceTimes>,
    /// The slowest of the window's slow frames, with the GPU's passes and the systems it waited
    /// on.
    pub worst_frame: Option<ProbeSlowFrame>,
    /// Every frame of the window slower than the stall the criteria allow, in order, each with
    /// the GPU's passes and the systems it waited on, as far as the probe kept them.
    #[serde(default)]
    pub stalled_frames: Vec<ProbeSlowFrame>,
    /// Every frame of the window in order, for the course of a scenario's frame times: where its
    /// slow frames fall, and what the main world, the render world and the GPU took on each.
    pub trace: Vec<FrameTrace>,
}

/// How long the editor took over its interface, frame by frame: the median of the whole
/// interface, of its trees and of the stage's overlays, and the slowest overlays of any frame.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct InterfaceTimes {
    pub ui: Millis,
    pub tree: Millis,
    pub overlay: Millis,
    pub overlay_worst: Millis,
}

impl FrameStats {
    /// The mean, the 1% low and the slowest frame, as a line of the log reads them.
    pub fn summary(&self) -> String {
        format!(
            "mean {:>5.0}  1% low {:>5.0} fps  p99 {:>5.1}  worst {:>6.1} ms  {} frames",
            self.mean.0, self.low.0, self.p99.0, self.worst.0, self.frames
        )
    }
}

fn median(values: impl Iterator<Item = Millis>) -> Option<Millis> {
    let mut values: Vec<f32> = values.map(|value| value.0).collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    Some(Millis(values[values.len() / 2]))
}

fn median_u32(values: impl Iterator<Item = u32>) -> Option<u32> {
    median_u64(values.map(u64::from)).map(|value| value as u32)
}

fn median_u64(values: impl Iterator<Item = u64>) -> Option<u64> {
    let mut values: Vec<u64> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

/// A frame of a scenario's window: how long it took, and how long the main world, the render world
/// and the GPU each took on it, where known.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FrameTrace {
    pub frame: u32,
    pub ms: Millis,
    pub cpu_main: Millis,
    pub cpu_render: Option<Millis>,
    pub gpu: Option<Millis>,
    pub props_drawn: Option<u32>,
}
