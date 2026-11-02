//! What the client and editor benches measure of the frames they watch, at each of the video
//! settings they are watched at, against the frame rate each must never fall below.

use engine::core::render::video::VideoPreferences;
use engine::core::units::Millis;

/// The video settings a bench watches at: every costly part of the picture at its best, and every
/// one off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Video {
    Highest,
    Lowest,
}

impl Video {
    pub const ALL: [Video; 2] = [Video::Highest, Video::Lowest];

    pub fn preferences(self) -> VideoPreferences {
        match self {
            Video::Highest => VideoPreferences::default(),
            Video::Lowest => VideoPreferences::lowest(),
        }
    }

    /// The frame rate no frame may fall below at these settings.
    pub fn floor(self) -> f64 {
        match self {
            Video::Highest => 90.0,
            Video::Lowest => 120.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Video::Highest => "highest",
            Video::Lowest => "lowest",
        }
    }
}

/// How long each of a run of frames took.
#[derive(Clone, Debug, Default)]
pub struct Frames(Vec<Millis>);

impl Frames {
    pub fn of(times: impl IntoIterator<Item = Millis>) -> Frames {
        Frames(times.into_iter().collect())
    }

    pub fn extend(&mut self, times: impl IntoIterator<Item = Millis>) {
        self.0.extend(times);
    }

    pub fn join(&mut self, other: &Frames) {
        self.0.extend_from_slice(&other.0);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Frames a second over the whole run.
    pub fn mean(&self) -> f64 {
        let lasted: f32 = self.0.iter().map(|time| time.0).sum();
        self.0.len() as f64 / f64::from(lasted.max(f32::EPSILON)) * 1000.0
    }

    /// The frame rate of the slowest hundredth of the frames.
    pub fn low(&self) -> f64 {
        let mut sorted: Vec<f32> = self.0.iter().map(|time| time.0).collect();
        sorted.sort_by(f32::total_cmp);
        let at = (sorted.len() as f64 * 0.99) as usize;
        sorted
            .get(at.min(sorted.len().saturating_sub(1)))
            .map_or(0.0, |&time| fps(time))
    }

    /// The frame rate of the slowest frame.
    pub fn slowest(&self) -> f64 {
        self.0
            .iter()
            .map(|time| time.0)
            .max_by(f32::total_cmp)
            .map_or(0.0, fps)
    }

    /// The mean, the 1% low and the slowest frame, as a report reads them.
    pub fn summary(&self) -> String {
        format!(
            "mean {:>5.0}  1% low {:>5.0}  slowest {:>5.0} fps",
            self.mean(),
            self.low(),
            self.slowest()
        )
    }
}

fn fps(time: f32) -> f64 {
    1000.0 / f64::from(time.max(f32::EPSILON))
}
