use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioCategory {
    Music,
    Ambience,
    Effects,
    Voice,
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
    ambience: AudioVolume,
    effects: AudioVolume,
    voice: AudioVolume,
}

impl AudioVolume {
    pub const SILENT: AudioVolume = AudioVolume(0.0);
    pub const BALANCED: AudioVolume = AudioVolume(0.5);
    pub const FULL: AudioVolume = AudioVolume(1.0);

    fn clamped(self) -> AudioVolume {
        AudioVolume(self.0.clamp(Self::SILENT.0, Self::FULL.0))
    }

    fn amplitude(self) -> f32 {
        self.0 / Self::BALANCED.0
    }
}

impl Default for AudioMix {
    fn default() -> AudioMix {
        AudioMix {
            master: AudioVolume::BALANCED,
            music: AudioVolume::BALANCED,
            ambience: AudioVolume::BALANCED,
            effects: AudioVolume::BALANCED,
            voice: AudioVolume::BALANCED,
        }
    }
}

impl AudioMix {
    pub fn level(&self, fader: AudioFader) -> AudioVolume {
        match fader {
            AudioFader::Master => self.master,
            AudioFader::Category(AudioCategory::Music) => self.music,
            AudioFader::Category(AudioCategory::Ambience) => self.ambience,
            AudioFader::Category(AudioCategory::Effects) => self.effects,
            AudioFader::Category(AudioCategory::Voice) => self.voice,
        }
    }

    pub fn set(&mut self, fader: AudioFader, volume: AudioVolume) {
        let level = match fader {
            AudioFader::Master => &mut self.master,
            AudioFader::Category(AudioCategory::Music) => &mut self.music,
            AudioFader::Category(AudioCategory::Ambience) => &mut self.ambience,
            AudioFader::Category(AudioCategory::Effects) => &mut self.effects,
            AudioFader::Category(AudioCategory::Voice) => &mut self.voice,
        };
        *level = volume.clamped();
    }

    pub fn gain(&self, category: AudioCategory) -> f32 {
        self.master.amplitude() * self.level(AudioFader::Category(category)).amplitude()
    }
}
