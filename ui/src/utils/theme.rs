use std::sync::RwLock;

use bevy_color::Color;

/// A themed color family: every family has exactly these five slots.
#[derive(Clone, Copy)]
pub struct Family {
    pub base: Color,
    pub on: Color,
    pub hover: Color,
    pub active: Color,
    pub border: Color,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub scrim_light: Color,
    pub scrim_dark: Color,
    pub error_solid: Family,
    pub error_soft: Family,
    pub info_soft: Family,
    pub info_solid: Family,
    pub primary: Family,
    pub secondary: Family,
    pub success_solid: Family,
    pub success_soft: Family,
    pub surface_canvas: Family,
    pub surface_inset: Family,
    pub surface_trough: Family,
    pub surface_elevated: Family,
    pub surface_floating: Family,
    pub surface_inverted: Family,
    pub neutral: Family,
}

/// The active theme. Components read it directly when they build their styles, so a theme is applied
/// at spawn — [`change_theme`] affects everything spawned after it, not already-built UI.
static ACTIVE: RwLock<Theme> = RwLock::new(crate::themes::light::THEME);

pub fn theme() -> Theme {
    *ACTIVE.read().expect("theme lock poisoned")
}

pub fn set_theme(theme: Theme) {
    *ACTIVE.write().expect("theme lock poisoned") = theme;
}
