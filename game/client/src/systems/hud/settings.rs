use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

use crate::systems::hud::Panel;

const KEY: &str = "rift.user_settings";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct ScreenPx(pub f32);

impl std::ops::Add<f32> for ScreenPx {
    type Output = ScreenPx;
    fn add(self, delta: f32) -> ScreenPx {
        ScreenPx(self.0 + delta)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct ScreenVec {
    pub x: ScreenPx,
    pub y: ScreenPx,
}

impl ScreenVec {
    pub fn to_vec2(self) -> Vec2 {
        Vec2::new(self.x.0, self.y.0)
    }

    pub fn from_vec2(v: Vec2) -> ScreenVec {
        ScreenVec {
            x: ScreenPx(v.x),
            y: ScreenPx(v.y),
        }
    }
}

pub const DEFAULT_SNAP: ScreenPx = ScreenPx(16.0);

#[derive(Serialize, Deserialize, Default)]
pub struct UserSettings {
    #[serde(default)]
    pub ui: UiSettings,
}

#[derive(Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_snap")]
    pub snap: ScreenPx,
    #[serde(default)]
    pub placements: Vec<(Panel, Placement)>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Placement {
    pub pos: ScreenVec,
    pub size: Option<ScreenVec>,
}

impl UserSettings {
    pub fn load() -> UserSettings {
        crate::core::platform::load(KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            crate::core::platform::save(KEY, &json);
        }
    }

    pub fn snap_grid(&self) -> f32 {
        self.ui.snap.0
    }

    pub fn placement(&self, panel: Panel) -> Option<Placement> {
        self.ui
            .placements
            .iter()
            .find(|(key, _)| *key == panel)
            .map(|(_, placement)| *placement)
    }

    pub fn set_placement(&mut self, panel: Panel, placement: Placement) {
        match self.ui.placements.iter_mut().find(|(key, _)| *key == panel) {
            Some(entry) => entry.1 = placement,
            None => self.ui.placements.push((panel, placement)),
        }
    }

    pub fn snapping_enabled(&self) -> bool {
        self.ui.snap.0 > 0.0
    }

    pub fn toggle_snapping(&mut self) {
        self.ui.snap = if self.ui.snap.0 > 0.0 {
            ScreenPx(0.0)
        } else {
            DEFAULT_SNAP
        };
    }
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            placements: Vec::new(),
        }
    }
}

fn default_snap() -> ScreenPx {
    DEFAULT_SNAP
}
