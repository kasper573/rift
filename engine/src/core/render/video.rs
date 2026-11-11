//! How the world's picture is shown, traded against the frame rate: the window it fills, whether
//! frames wait on the display, and each costly part of its finish at a quality the player picks,
//! all at once by a preset or one by one, all at their best unless picked otherwise. The game's
//! settings and the editor pick them through the same controls.

use bevy::anti_alias::smaa::SmaaPreset;
use bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::ExtractResource;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};
use ui::controls;
use ui::egui::Ui;

use crate::core::units::Meters;

#[derive(Resource, ExtractResource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[extract_app(RenderApp)]
#[serde(default)]
pub struct VideoPreferences {
    /// The window fills the screen it is on.
    pub fullscreen: bool,
    /// Each frame waits for the display's refresh. Off unless asked for, so frames go out without
    /// waiting on it: a desktop may hold that wait up, as some do for tens of milliseconds at every
    /// mouse press and release, and the whole frame would wait too.
    pub vsync: bool,
    /// The sun's and the moon's.
    pub shadows: ShadowQuality,
    /// How the creases between things darken where little light reaches.
    pub ambient_occlusion: AmbientOcclusionQuality,
    /// The sun's shafts through the air.
    pub light_shafts: bool,
    pub anti_aliasing: AntiAliasingQuality,
    /// What the light catches glowing over its rim.
    pub bloom: bool,
    /// How far around the player the world's props, covers and lights stand.
    pub view_distance: ViewDistance,
    /// How thickly the blades the ground's covers grow stand, and how far off.
    pub grass: GrassQuality,
    /// How close a prop still draws its model rather than its picture.
    pub detail: PropDetail,
}

impl Default for VideoPreferences {
    fn default() -> VideoPreferences {
        VideoPreferences {
            fullscreen: false,
            vsync: false,
            shadows: ShadowQuality::default(),
            ambient_occlusion: AmbientOcclusionQuality::default(),
            light_shafts: true,
            anti_aliasing: AntiAliasingQuality::default(),
            bloom: true,
            view_distance: ViewDistance::default(),
            grass: GrassQuality::default(),
            detail: PropDetail::default(),
        }
    }
}

impl VideoPreferences {
    /// These preferences with every part of the finish at `quality`, the window as it is.
    pub fn at(self, quality: Quality) -> VideoPreferences {
        let (shadows, ambient_occlusion, anti_aliasing, view_distance, grass, detail) =
            match quality {
                Quality::Lowest => (
                    ShadowQuality::Off,
                    AmbientOcclusionQuality::Off,
                    AntiAliasingQuality::Off,
                    ViewDistance::Near,
                    GrassQuality::Low,
                    PropDetail::Low,
                ),
                Quality::Low => (
                    ShadowQuality::Low,
                    AmbientOcclusionQuality::Low,
                    AntiAliasingQuality::Low,
                    ViewDistance::Middle,
                    GrassQuality::Medium,
                    PropDetail::Medium,
                ),
                Quality::Medium => (
                    ShadowQuality::Low,
                    AmbientOcclusionQuality::Medium,
                    AntiAliasingQuality::Medium,
                    ViewDistance::Middle,
                    GrassQuality::Medium,
                    PropDetail::Medium,
                ),
                Quality::High => (
                    ShadowQuality::High,
                    AmbientOcclusionQuality::High,
                    AntiAliasingQuality::High,
                    ViewDistance::Far,
                    GrassQuality::High,
                    PropDetail::High,
                ),
                Quality::Max => (
                    ShadowQuality::High,
                    AmbientOcclusionQuality::High,
                    AntiAliasingQuality::Ultra,
                    ViewDistance::Far,
                    GrassQuality::High,
                    PropDetail::High,
                ),
            };
        VideoPreferences {
            shadows,
            ambient_occlusion,
            light_shafts: quality >= Quality::High,
            anti_aliasing,
            bloom: quality >= Quality::Medium,
            view_distance,
            grass,
            detail,
            ..self
        }
    }

    /// The preset every part of the finish stands at, if any.
    pub fn quality(self) -> Option<Quality> {
        Quality::VARIANTS
            .iter()
            .copied()
            .find(|&quality| self.at(quality) == self)
    }
}

/// Every costly part of the finish picked at once, from all of them off to all at their best.
#[derive(
    Serialize,
    Deserialize,
    VariantArray,
    AsRefStr,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub enum Quality {
    Lowest,
    Low,
    Medium,
    High,
    Max,
}

/// What the preset picker shows when the parts stand at no preset.
pub const CUSTOM_QUALITY: &str = "Custom";

/// What the settings' toggles read, as the interface shows them and a bench finds them by.
pub const FULLSCREEN_TOGGLE: &str = "Fullscreen";
pub const VSYNC_TOGGLE: &str = "VSync";
pub const BLOOM_TOGGLE: &str = "Bloom";
pub const LIGHT_SHAFTS_TOGGLE: &str = "Light shafts";

/// The controls the player picks `video` with: its preset first, then every part on its own.
pub fn controls(ui: &mut Ui, video: &mut VideoPreferences) {
    controls::form(ui, "quality", |ui| {
        let label = ui.label("Quality");
        let current = video.quality();
        let mut picked = current;
        controls::combo("quality")
            .selected_text(
                current
                    .as_ref()
                    .map_or(CUSTOM_QUALITY, |quality| quality.as_ref()),
            )
            .show_ui(ui, |ui| {
                for &quality in Quality::VARIANTS {
                    ui.selectable_value(&mut picked, Some(quality), quality.as_ref());
                }
            })
            .response
            .labelled_by(label.id);
        if let Some(quality) = picked.filter(|&quality| Some(quality) != current) {
            *video = video.at(quality);
        }
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.checkbox(&mut video.fullscreen, FULLSCREEN_TOGGLE);
    ui.checkbox(&mut video.vsync, VSYNC_TOGGLE).on_hover_text(
        "Each frame waits for the display's refresh: less heat and power, a little more delay",
    );
    ui.checkbox(&mut video.bloom, BLOOM_TOGGLE)
        .on_hover_text("What the light catches glows over its rim");
    ui.checkbox(&mut video.light_shafts, LIGHT_SHAFTS_TOGGLE)
        .on_hover_text("The sun's rays show in the air");
    ui.add_space(4.0);
    controls::form(ui, "video", |ui| {
        let label = ui.label("Shadows");
        if let Some(shadows) = controls::variant_picker(ui, "shadows", video.shadows, label.id) {
            video.shadows = shadows;
        }
        ui.end_row();
        let label = ui.label("Ambient occlusion");
        let current = video.ambient_occlusion;
        if let Some(occlusion) =
            controls::variant_picker(ui, "ambient occlusion", current, label.id)
        {
            video.ambient_occlusion = occlusion;
        }
        ui.end_row();
        let label = ui.label("Anti-aliasing");
        let current = video.anti_aliasing;
        if let Some(anti_aliasing) =
            controls::variant_picker(ui, "anti-aliasing", current, label.id)
        {
            video.anti_aliasing = anti_aliasing;
        }
        ui.end_row();
        let label = ui.label("View distance");
        let current = video.view_distance;
        if let Some(distance) = controls::variant_picker(ui, "view distance", current, label.id) {
            video.view_distance = distance;
        }
        ui.end_row();
        let label = ui.label("Grass");
        if let Some(grass) = controls::variant_picker(ui, "grass", video.grass, label.id) {
            video.grass = grass;
        }
        ui.end_row();
        let label = ui.label("Detail");
        if let Some(detail) = controls::variant_picker(ui, "detail", video.detail, label.id) {
            video.detail = detail;
        }
        ui.end_row();
    });
}

/// The primary window fills its screen and waits on its refresh as the preferences ask.
pub(super) fn show_window(
    video: Res<VideoPreferences>,
    window: Option<Single<&mut Window, With<PrimaryWindow>>>,
) {
    let Some(mut window) = window else {
        return;
    };
    let mode = if video.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    let present_mode = if video.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    if window.mode != mode {
        window.mode = mode;
    }
    if window.present_mode != present_mode {
        window.present_mode = present_mode;
    }
}

#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ShadowQuality {
    Off,
    /// Shadows from a single cascade, close to the camera.
    Low,
    /// Shadows as far as the camera sees the ground, in cascades the finer the nearer it.
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

/// How far around the player the world's props, covers and lights stand, as a share of as far as
/// the game frames its camera to see.
#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ViewDistance {
    Near,
    Middle,
    #[default]
    Far,
}

impl ViewDistance {
    pub fn share(self) -> f32 {
        match self {
            ViewDistance::Near => 0.5,
            ViewDistance::Middle => 0.75,
            ViewDistance::Far => 1.0,
        }
    }

    /// How far off the eye the far field's props still stand, or as far as the ground goes.
    pub fn horizon(self) -> Option<Meters> {
        match self {
            ViewDistance::Near => Some(Meters(300.0)),
            ViewDistance::Middle => Some(Meters(600.0)),
            ViewDistance::Far => None,
        }
    }
}

/// How thickly the blades the ground's covers grow stand, and how far off, as shares of all a
/// cover grows and of as far as the view still shows its blades.
#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum GrassQuality {
    Low,
    Medium,
    #[default]
    High,
}

impl GrassQuality {
    /// The share of a cover's blades that stand.
    pub fn density(self) -> f32 {
        self.share()
    }

    /// The share of as far as the view shows a cover's blades that they stand within.
    pub fn range(self) -> f32 {
        self.share()
    }

    fn share(self) -> f32 {
        match self {
            GrassQuality::Low => 0.25,
            GrassQuality::Medium => 0.5,
            GrassQuality::High => 1.0,
        }
    }
}

/// How close a prop still draws its model rather than its picture: the width on screen below
/// which its impostor stands in for it, as a multiple of the picture's own width, past which the
/// picture is drawn larger than it was baked.
#[derive(
    Serialize, Deserialize, VariantArray, AsRefStr, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum PropDetail {
    Low,
    Medium,
    #[default]
    High,
}

impl PropDetail {
    pub fn impostor_widths(self) -> f32 {
        match self {
            PropDetail::Low => 4.0,
            PropDetail::Medium => 2.0,
            PropDetail::High => 1.0,
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
