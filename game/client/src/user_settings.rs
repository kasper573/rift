//! Session-spanning user preferences: a serde god-struct persisted as JSON, on the native
//! filesystem and in the browser's localStorage (via js/rift_storage.js). Today it carries only
//! the UI layout; new preference groups slot in as further fields on [`UserSettings`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The snap grid in pixels when snapping is enabled; 0 disables it.
pub const DEFAULT_SNAP: f32 = 16.0;

#[derive(Serialize, Deserialize, Default)]
pub struct UserSettings {
    #[serde(default)]
    pub ui: UiSettings,
}

#[derive(Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default = "default_snap")]
    pub snap: f32,
    #[serde(default)]
    pub placements: HashMap<String, Placement>,
}

/// A persisted on-screen rectangle: top-left position, and a size for resizable windows.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Placement {
    pub pos: (f32, f32),
    pub size: Option<(f32, f32)>,
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
    pub fn snap(&self, value: f32) -> f32 {
        if self.ui.snap <= 0.0 {
            value
        } else {
            (value / self.ui.snap).round() * self.ui.snap
        }
    }

    pub fn placement(&self, id: &str) -> Option<Placement> {
        self.ui.placements.get(id).copied()
    }

    pub fn set_placement(&mut self, id: &str, placement: Placement) {
        self.ui.placements.insert(id.to_owned(), placement);
    }

    pub fn snapping_enabled(&self) -> bool {
        self.ui.snap > 0.0
    }

    pub fn toggle_snapping(&mut self) {
        self.ui.snap = if self.ui.snap > 0.0 {
            0.0
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

fn default_snap() -> f32 {
    DEFAULT_SNAP
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
mod imp {
    // The contract with js/rift_storage.js (appended to mq_js_bundle.js at staging): load stages
    // the stored value and returns its byte length, or -1 when absent; read copies the staged
    // bytes into the buffer. The wasm-side counterpart is mirrored on src/platform.rs's pattern.
    unsafe extern "C" {
        fn rift_storage_load() -> i32;
        fn rift_storage_read(pointer: *mut u8);
        fn rift_storage_save(pointer: *const u8, length: usize);
    }

    pub fn load() -> Option<String> {
        let length = unsafe { rift_storage_load() };
        if length < 0 {
            return None;
        }
        let mut buffer = vec![0u8; length as usize];
        unsafe { rift_storage_read(buffer.as_mut_ptr()) };
        String::from_utf8(buffer).ok()
    }

    pub fn save(json: &str) {
        unsafe { rift_storage_save(json.as_ptr(), json.len()) };
    }
}
