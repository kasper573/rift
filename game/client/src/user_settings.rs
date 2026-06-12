use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A window-space (logical) pixel, the unit bevy UI `Val::Px` and `Window::cursor_position` speak —
/// distinct from the world-render pixels the camera draws in (a whole-number letterbox scale apart).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Logical(pub f32);

impl std::ops::Add<f32> for Logical {
    type Output = Logical;
    fn add(self, delta: f32) -> Logical {
        Logical(self.0 + delta)
    }
}

/// The snap grid when snapping is enabled; 0 disables it.
pub const DEFAULT_SNAP: Logical = Logical(16.0);

#[derive(Serialize, Deserialize, Default)]
pub struct UserSettings {
    #[serde(default)]
    pub ui: UiSettings,
}

#[derive(Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_snap")]
    pub snap: Logical,
    #[serde(default)]
    pub placements: HashMap<String, Placement>,
}

/// A persisted on-screen rectangle: top-left position, and a size for resizable windows.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Placement {
    pub pos: (Logical, Logical),
    pub size: Option<(Logical, Logical)>,
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

    /// Rounds a coordinate to the snap grid; a no-op while snapping is disabled.
    pub fn snap(&self, value: Logical) -> Logical {
        let grid = self.ui.snap.0;
        if grid <= 0.0 {
            value
        } else {
            Logical((value.0 / grid).round() * grid)
        }
    }

    pub fn placement(&self, id: &str) -> Option<Placement> {
        self.ui.placements.get(id).copied()
    }

    pub fn set_placement(&mut self, id: &str, placement: Placement) {
        self.ui.placements.insert(id.to_owned(), placement);
    }

    pub fn snapping_enabled(&self) -> bool {
        self.ui.snap.0 > 0.0
    }

    pub fn toggle_snapping(&mut self) {
        self.ui.snap = if self.ui.snap.0 > 0.0 {
            Logical(0.0)
        } else {
            DEFAULT_SNAP
        };
    }
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            placements: HashMap::new(),
        }
    }
}

fn default_snap() -> Logical {
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
