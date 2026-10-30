//! How much of the world's picture the player has drawn, traded against the frame rate: each costly
//! part of its finish at a quality the player picks, all at their best unless picked otherwise.

use bevy::anti_alias::smaa::SmaaPreset;
use bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};

#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct VideoPreferences {
    /// The sun's and the moon's.
    pub shadows: ShadowQuality,
    /// How the creases between things darken where little light reaches.
    pub ambient_occlusion: AmbientOcclusionQuality,
    /// The sun's shafts through the air.
    pub light_shafts: bool,
    pub anti_aliasing: AntiAliasingQuality,
    /// What the light catches glowing over its rim.
    pub bloom: bool,
}

impl Default for VideoPreferences {
    fn default() -> VideoPreferences {
        VideoPreferences {
            shadows: ShadowQuality::default(),
            ambient_occlusion: AmbientOcclusionQuality::default(),
            light_shafts: true,
            anti_aliasing: AntiAliasingQuality::default(),
            bloom: true,
        }
    }
}

#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ShadowQuality {
    Off,
    /// Hard shadows from a single cascade.
    Low,
    /// Soft shadows from two cascades where the platform draws two, the finer near the camera.
    #[default]
    High,
}

#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum AmbientOcclusionQuality {
    Off,
    Low,
    Medium,
    #[default]
    High,
}

impl AmbientOcclusionQuality {
    pub fn level(self) -> Option<ScreenSpaceAmbientOcclusionQualityLevel> {
        match self {
            AmbientOcclusionQuality::Off => None,
            AmbientOcclusionQuality::Low => Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
            AmbientOcclusionQuality::Medium => {
                Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium)
            }
            AmbientOcclusionQuality::High => Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
        }
    }
}

/// How far the finished picture's edges are smoothed, by morphological anti-aliasing.
#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum AntiAliasingQuality {
    Off,
    Low,
    Medium,
    High,
    #[default]
    Ultra,
}

impl AntiAliasingQuality {
    pub fn preset(self) -> Option<SmaaPreset> {
        match self {
            AntiAliasingQuality::Off => None,
            AntiAliasingQuality::Low => Some(SmaaPreset::Low),
            AntiAliasingQuality::Medium => Some(SmaaPreset::Medium),
            AntiAliasingQuality::High => Some(SmaaPreset::High),
            AntiAliasingQuality::Ultra => Some(SmaaPreset::Ultra),
        }
    }
}
