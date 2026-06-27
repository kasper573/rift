use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use ui::button::intent as button_intent;
use ui::{Activate, ButtonSize, button_styled};

use super::{Settings, WindowDef};

const KEY: &str = "rift.user_settings";

#[derive(Component, Default, Clone)]
struct SnappingButton;

inventory::submit! {
    WindowDef {
        id: "Settings",
        title: "Settings",
        toggle: KeyCode::KeyO,
        keybind: "O",
        icon: "icons/misc/gear.png",
        order: 3,
        content,
        sync: sync_snapping,
    }
}

fn content() -> Box<dyn Scene> {
    Box::new(bsn! {
        {button_styled(button_intent::PRIMARY, ButtonSize::Md, "ui snapping disabled")}
        SnappingButton
        on(|_: On<Activate>, mut commands: Commands| {
            commands.queue(toggle_snapping);
        })
    })
}

fn sync_snapping(world: &mut World) {
    let label = if world.resource::<Settings>().0.snapping_enabled() {
        "ui snapping enabled"
    } else {
        "ui snapping disabled"
    };
    let texts: Vec<Entity> = world
        .query_filtered::<&Children, With<SnappingButton>>()
        .iter(world)
        .flat_map(|children| children.iter())
        .collect();
    for entity in texts {
        if let Some(mut text) = world.get_mut::<Text>(entity) {
            text.0 = label.to_owned();
        }
    }
}

fn toggle_snapping(world: &mut World) {
    let mut settings = world.resource_mut::<Settings>();
    settings.0.toggle_snapping();
    settings.0.save();
}

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
    /// Each docked widget's saved position, by id ("character", "effects", or a window's id for its
    /// launcher).
    #[serde(default)]
    pub widgets: Vec<(String, ScreenVec)>,
    /// Each open window's saved position and size, by the window's id.
    #[serde(default)]
    pub windows: Vec<(String, Placement)>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Placement {
    pub pos: ScreenVec,
    pub size: ScreenVec,
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

    pub fn widget_pos(&self, id: &str) -> Option<ScreenVec> {
        self.ui
            .widgets
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, pos)| *pos)
    }

    pub fn set_widget_pos(&mut self, id: &str, pos: ScreenVec) {
        match self.ui.widgets.iter_mut().find(|(key, _)| key == id) {
            Some(entry) => entry.1 = pos,
            None => self.ui.widgets.push((id.to_owned(), pos)),
        }
    }

    pub fn window_placement(&self, id: &str) -> Option<Placement> {
        self.ui
            .windows
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, placement)| *placement)
    }

    pub fn set_window_placement(&mut self, id: &str, placement: Placement) {
        match self.ui.windows.iter_mut().find(|(key, _)| key == id) {
            Some(entry) => entry.1 = placement,
            None => self.ui.windows.push((id.to_owned(), placement)),
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
            widgets: Vec::new(),
            windows: Vec::new(),
        }
    }
}

fn default_snap() -> ScreenPx {
    DEFAULT_SNAP
}
