//! How the world's picture is shown, traded against the frame rate: the window it fills, whether
//! frames wait on the display, and each costly part of its finish at a quality the player picks,
//! all at their best unless picked otherwise. The game's settings and the editor pick them through
//! the same controls.

use bevy::anti_alias::smaa::SmaaPreset;
use bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, VariantArray};
use ui::controls;
use ui::egui::Ui;

#[derive(Resource, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Which way the world's props are drawn while the GPU's way replaces the old one.
    pub world_renderer: WorldRenderer,
}

/// The way the world's props are drawn: each as entities of its own and merged meshes, or from
/// arenas on the GPU.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldRenderer {
    #[default]
    Legacy,
    Gpu,
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
            world_renderer: WorldRenderer::default(),
        }
    }
}

impl VideoPreferences {
    /// Every costly part of the finish off, in a window.
    pub fn lowest() -> VideoPreferences {
        VideoPreferences {
            fullscreen: false,
            vsync: false,
            shadows: ShadowQuality::Off,
            ambient_occlusion: AmbientOcclusionQuality::Off,
            light_shafts: false,
            anti_aliasing: AntiAliasingQuality::Off,
            bloom: false,
            view_distance: ViewDistance::Near,
            world_renderer: WorldRenderer::default(),
        }
    }
}

/// What the settings' toggles read, as the interface shows them and a bench finds them by.
pub const FULLSCREEN_TOGGLE: &str = "Fullscreen";
pub const VSYNC_TOGGLE: &str = "VSync";
pub const BLOOM_TOGGLE: &str = "Bloom";
pub const LIGHT_SHAFTS_TOGGLE: &str = "Light shafts";

/// The controls the player picks `video` with.
pub fn controls(ui: &mut Ui, video: &mut VideoPreferences) {
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
    /// Hard shadows from a single cascade.
    Low,
    /// Soft shadows from two cascades, the finer near the camera.
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
