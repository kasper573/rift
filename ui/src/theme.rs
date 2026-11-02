//! The colors and text styles the parts of an interface take by their role, drawn from the inks.

use egui::{Color32, FontFamily, RichText};

use crate::face::Face;
use crate::ink::{self, Ground, Prose};

pub const TEXT: Color32 = ink::INK;
pub const MUTED: Color32 = ink::INK2;
pub const FAINT: Color32 = ink::INK3;
pub const CODE: Color32 = ink::INK2;
pub const DANGER: Color32 = Prose::Danger.on(Ground::Paper);
pub const OK: Color32 = ink::VERDIGRIS;
pub const WARN: Color32 = ink::OCHRE;
pub const ACCENT: Color32 = ink::RUBRIC;
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(37, 29, 9, 51);
pub const SELECTION: Color32 = Color32::from_rgba_premultiplied(51, 40, 12, 71);
pub const BORDER: Color32 = Color32::from_rgba_premultiplied(15, 11, 7, 89);
pub const INPUT: Color32 = Color32::from_rgba_premultiplied(2, 1, 1, 10);
pub const PANEL: Color32 = ink::CREAM;
pub const RAISED: Color32 = ink::LIGHT;
/// Behind a picture shown at its own pixels: a map, a sheet, a frame.
pub const CANVAS: Color32 = Color32::from_rgb(219, 207, 178);
/// What a drag or a tool may not do here.
pub const BLOCKED: Color32 = ink::RUBRIC;
/// Where a drag lands.
pub const DROP: Color32 = ink::AZURE;

pub fn medium() -> FontFamily {
    Face::BookMedium.family()
}

pub fn ok(text: impl Into<String>) -> RichText {
    RichText::new(text).color(OK)
}

pub fn warn(text: impl Into<String>) -> RichText {
    RichText::new(text).color(WARN)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn faint(text: impl Into<String>) -> RichText {
    RichText::new(text).color(FAINT)
}

pub fn code(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(CODE)
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(medium()).color(TEXT)
}
