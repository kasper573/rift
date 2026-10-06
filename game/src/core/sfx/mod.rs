pub mod playback;

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;
use crate::core::math::Rng;

pub use crate::data::sfx::Id as SfxId;

pub struct SfxDef {
    pub src: AssetRef,
    pub category: AudioCategory,
    pub volume: SfxScalar,
    pub pitch: SfxScalar,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCategory {
    Music,
    Voice,
    Effects,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct AudioVolume(pub f32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFader {
    Master,
    Category(AudioCategory),
}

#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct AudioMix {
    master: AudioVolume,
    music: AudioVolume,
    voice: AudioVolume,
    effects: AudioVolume,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxScalar {
    Fixed(f32),
    Random(f32, f32),
}

impl Default for SfxScalar {
    fn default() -> SfxScalar {
        SfxScalar::Fixed(1.0)
    }
}

impl SfxScalar {
    pub fn resolve(self, rng: &mut Rng) -> f32 {
        match self {
            SfxScalar::Fixed(value) => value,
            SfxScalar::Random(min, max) => min + rng.rand_float() * (max - min),
        }
    }
}

impl AudioVolume {
    pub const SILENT: AudioVolume = AudioVolume(0.0);
    pub const FULL: AudioVolume = AudioVolume(1.0);

    fn clamped(self) -> AudioVolume {
        AudioVolume(self.0.clamp(Self::SILENT.0, Self::FULL.0))
    }

    // Loudness is heard logarithmically, so squaring spreads it evenly along a slider.
    fn amplitude(self) -> f32 {
        self.0 * self.0
    }
}

impl Default for AudioMix {
    fn default() -> AudioMix {
        AudioMix {
            master: AudioVolume::FULL,
            music: AudioVolume::FULL,
            voice: AudioVolume::FULL,
            effects: AudioVolume::FULL,
        }
    }
}

impl AudioMix {
    pub fn level(&self, fader: AudioFader) -> AudioVolume {
        match fader {
            AudioFader::Master => self.master,
            AudioFader::Category(AudioCategory::Music) => self.music,
            AudioFader::Category(AudioCategory::Voice) => self.voice,
            AudioFader::Category(AudioCategory::Effects) => self.effects,
        }
    }

    pub fn set(&mut self, fader: AudioFader, volume: AudioVolume) {
        let level = match fader {
            AudioFader::Master => &mut self.master,
            AudioFader::Category(AudioCategory::Music) => &mut self.music,
            AudioFader::Category(AudioCategory::Voice) => &mut self.voice,
            AudioFader::Category(AudioCategory::Effects) => &mut self.effects,
        };
        *level = volume.clamped();
    }

    pub fn gain(&self, category: AudioCategory) -> f32 {
        self.master.amplitude() * self.level(AudioFader::Category(category)).amplitude()
    }
}
