//! Every frame the client draws, measured: a ring of the latest frames a driver reads through
//! the probe, cursoring on from the last frame it saw, told how many it missed when the ring has
//! wrapped past its cursor.

use std::collections::VecDeque;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::render::sample::{GpuReport, RenderReport};
use crate::core::units::{Bytes, Millis};

/// What one frame cost and showed: how long it took, the main world's and the render world's
/// time on it, the GPU's once that has come back, what the view took in that was not yet stood
/// at its wanted detail, and what was drawn and sent to the GPU.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct FrameSample {
    pub frame: u32,
    pub dt_ms: Millis,
    pub cpu_main_ms: Millis,
    /// Unknown until the render world has drawn the frame.
    pub cpu_render_ms: Option<Millis>,
    /// Unknown until the GPU's timings for the frame have come back, or for good where the
    /// adapter keeps no timestamps.
    pub gpu_ms: Option<Millis>,
    /// Chunks of ground and scenery the view takes in that stand coarser than wanted, or not yet.
    pub missing: u32,
    pub draws: u32,
    pub instances: u32,
    pub entities: u32,
    pub visible_chunks: u32,
    pub upload_bytes: Bytes,
    /// Vfx materials painted anew this frame.
    pub vfx_writes: u32,
    /// Bodies posed anew this frame: set playing, turned or eased between clips.
    pub posed: u32,
    /// Plans of what the scenery stands begun this frame, as its sight moved out of the last.
    pub plans: u32,
}

/// The frames after the one a driver last saw, and how many of those the ring had already let go.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeFrames {
    pub samples: Vec<FrameSample>,
    pub dropped: u32,
    /// The frames after the one a driver last saw that took longer than [`HITCH`], each with the
    /// GPU's time by pass and the systems it waited on.
    pub slow: Vec<ProbeSlowFrame>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeSlowFrame {
    pub frame: u32,
    pub dt_ms: Millis,
    pub passes: Vec<ProbePass>,
    /// The systems of the main and the render world that ran longest while the frame was waited
    /// on: those of the frames before it, whose work its time measures; none where nothing timed
    /// them.
    pub systems: Vec<ProbeSystem>,
}

/// A system and how long it ran over a frame.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeSystem {
    pub name: String,
    pub ms: Millis,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbePass {
    pub name: String,
    pub gpu_ms: Millis,
}

/// The latest frames, one sample each in the order drawn, and the latest of them that were slow.
#[derive(Resource, Debug)]
pub struct FrameSamples {
    ring: VecDeque<FrameSample>,
    capacity: usize,
    /// How many frames a sample waits in the ring before it is read, for its late fields to land.
    settle: u32,
    slow: VecDeque<ProbeSlowFrame>,
    /// The systems that ran longest over each of the latest [`WAITED_ON`] frames, the latest last.
    latest_systems: VecDeque<Vec<ProbeSystem>>,
}

impl Default for FrameSamples {
    fn default() -> FrameSamples {
        FrameSamples::new(FRAMES_KEPT, SETTLE_FRAMES)
    }
}

/// How many frames the ring holds: some half a minute of frames however fast they come.
pub const FRAMES_KEPT: usize = 8192;
/// How many frames the render world and the GPU may trail the main world by before a sample is
/// read without them.
pub const SETTLE_FRAMES: u32 = 8;
/// How long a frame may take before it is kept as slow, with what it waited on.
pub const HITCH: Millis = Millis(20.0);
/// How many slow frames are kept: the latest, however long ago.
pub const SLOW_KEPT: usize = 512;
/// How many of the systems that ran longest a slow frame names.
pub const LONGEST_SYSTEMS: usize = 3;
/// How many frames before a slow frame it may have waited on: its own time measures the work of
/// the frame before it, which waited on the render world drawing the one before that.
const WAITED_ON: usize = 3;

impl FrameSamples {
    pub fn new(capacity: usize, settle: u32) -> FrameSamples {
        FrameSamples {
            ring: VecDeque::with_capacity(capacity),
            capacity,
            settle,
            slow: VecDeque::new(),
            latest_systems: VecDeque::new(),
        }
    }

    /// Keeps `sample` as the latest frame, with the systems that ran longest over it, letting the
    /// oldest go once the ring is full.
    pub fn push(&mut self, sample: FrameSample, systems: Vec<ProbeSystem>) {
        if self.ring.len() == self.capacity {
            self.ring.pop_front();
        }
        self.ring.push_back(sample);
        if sample.dt_ms.0 > HITCH.0 {
            let mut waited_on: Vec<ProbeSystem> =
                self.latest_systems.iter().flatten().cloned().collect();
            waited_on.sort_by(|a, b| b.ms.0.total_cmp(&a.ms.0));
            waited_on.truncate(LONGEST_SYSTEMS);
            if self.slow.len() == SLOW_KEPT {
                self.slow.pop_front();
            }
            self.slow.push_back(ProbeSlowFrame {
                frame: sample.frame,
                dt_ms: sample.dt_ms,
                passes: Vec::new(),
                systems: waited_on,
            });
        }
        if self.latest_systems.len() == WAITED_ON {
            self.latest_systems.pop_front();
        }
        self.latest_systems.push_back(systems);
    }

    /// Fills in what the render world measured of a frame, if it is still in the ring.
    pub fn drawn(&mut self, report: &RenderReport) {
        if let Some(sample) = self.sample_mut(report.frame) {
            sample.cpu_render_ms = Some(report.cpu_render_ms);
            sample.draws = report.draws;
            sample.instances = report.instances;
            sample.upload_bytes = report.upload_bytes;
        }
    }

    /// Fills in what the GPU measured of a frame, if it is still in the ring.
    pub fn timed(&mut self, report: &GpuReport) {
        if let Some(sample) = self.sample_mut(report.frame) {
            sample.gpu_ms = Some(report.gpu_ms);
        }
        if let Some(slow) = self
            .slow
            .iter_mut()
            .rev()
            .find(|slow| slow.frame == report.frame)
        {
            slow.passes = report
                .passes
                .iter()
                .map(|(name, gpu_ms)| ProbePass {
                    name: name.clone(),
                    gpu_ms: *gpu_ms,
                })
                .collect();
        }
    }

    /// The settled frames after `since`, and how many frames after it the ring has let go.
    pub fn since(&self, since: u32) -> ProbeFrames {
        let readable = self
            .ring
            .back()
            .map_or(0, |latest| latest.frame.saturating_sub(self.settle));
        let samples: Vec<FrameSample> = self
            .ring
            .iter()
            .filter(|sample| sample.frame > since && sample.frame <= readable)
            .copied()
            .collect();
        let dropped = self
            .ring
            .front()
            .map_or(0, |oldest| oldest.frame.saturating_sub(since + 1));
        ProbeFrames {
            samples,
            dropped,
            slow: self
                .slow
                .iter()
                .filter(|slow| slow.frame > since && slow.frame <= readable)
                .cloned()
                .collect(),
        }
    }

    pub fn latest(&self) -> Option<&FrameSample> {
        self.ring.back()
    }

    fn sample_mut(&mut self, frame: u32) -> Option<&mut FrameSample> {
        let oldest = self.ring.front()?.frame;
        let index = usize::try_from(frame.checked_sub(oldest)?).ok()?;
        self.ring
            .get_mut(index)
            .filter(|sample| sample.frame == frame)
    }
}
