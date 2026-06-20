use serde::{Deserialize, Serialize};

use crate::hud::Panel;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct ScreenPx(pub f32);

impl std::ops::Add<f32> for ScreenPx {
    type Output = ScreenPx;
    fn add(self, delta: f32) -> ScreenPx {
        ScreenPx(self.0 + delta)
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
    pub pos: (ScreenPx, ScreenPx),
    pub size: Option<(ScreenPx, ScreenPx)>,
}

impl UserSettings {
    pub fn load() -> UserSettings {
        imp::load()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            imp::save(&json);
        }
    }

    pub fn snap(&self, value: ScreenPx) -> ScreenPx {
        let grid = self.ui.snap.0;
        if grid <= 0.0 {
            value
        } else {
            ScreenPx((value.0 / grid).round() * grid)
        }
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

mod imp {
    use std::path::PathBuf;

    pub fn load() -> Option<String> {
        std::fs::read_to_string(path()?).ok()
    }

    pub fn save(json: &str) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(error) = std::fs::write(&path, json) {
            eprintln!(
                "could not save user settings to {}: {error}",
                path.display()
            );
        }
    }

    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("rift").join("user_settings.json"))
    }
}
