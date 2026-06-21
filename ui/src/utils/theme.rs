use bevy_color::Color;
use bevy_ecs::resource::Resource;

/// A themed color family: every family has exactly these five slots. The generic parameter is the
/// stored color in a [`Theme`] (`Color`) and the resolvable handle in [`color`] (`ColorVar`).
#[derive(Clone, Copy)]
pub struct Family<T> {
    pub base: T,
    pub on: T,
    pub hover: T,
    pub active: T,
    pub border: T,
}

#[derive(Clone, Copy)]
pub enum ColorSlot {
    Base,
    On,
    Hover,
    Active,
    Border,
}

impl ColorSlot {
    fn read(self, family: &Family<Color>) -> Color {
        match self {
            ColorSlot::Base => family.base,
            ColorSlot::On => family.on,
            ColorSlot::Hover => family.hover,
            ColorSlot::Active => family.active,
            ColorSlot::Border => family.border,
        }
    }
}

#[derive(Clone, Copy)]
enum Source {
    Slot {
        family: fn(&Theme) -> &Family<Color>,
        slot: ColorSlot,
    },
    Direct(fn(&Theme) -> Color),
}

/// A theme-relative color: a family slot, or a standalone one-shot color. Resolved against the
/// active [`Theme`] when styling is applied.
#[derive(Clone, Copy)]
pub struct ColorVar(Source);

impl ColorVar {
    const fn slot(family: fn(&Theme) -> &Family<Color>, slot: ColorSlot) -> ColorVar {
        ColorVar(Source::Slot { family, slot })
    }

    const fn direct(read: fn(&Theme) -> Color) -> ColorVar {
        ColorVar(Source::Direct(read))
    }

    pub fn resolve(self, theme: &Theme) -> Color {
        match self.0 {
            Source::Slot { family, slot } => slot.read(family(theme)),
            Source::Direct(read) => read(theme),
        }
    }
}

const fn vars(family: fn(&Theme) -> &Family<Color>) -> Family<ColorVar> {
    Family {
        base: ColorVar::slot(family, ColorSlot::Base),
        on: ColorVar::slot(family, ColorSlot::On),
        hover: ColorVar::slot(family, ColorSlot::Hover),
        active: ColorVar::slot(family, ColorSlot::Active),
        border: ColorVar::slot(family, ColorSlot::Border),
    }
}

#[derive(Resource, Clone, Copy)]
#[rustfmt::skip]
pub struct Theme {
    pub scrim_light: Color,
    pub scrim_dark: Color,
    pub error_solid: Family<Color>,
    pub error_soft: Family<Color>,
    pub info_soft: Family<Color>,
    pub info_solid: Family<Color>,
    pub primary: Family<Color>,
    pub secondary: Family<Color>,
    pub success_solid: Family<Color>,
    pub success_soft: Family<Color>,
    pub surface_canvas: Family<Color>,
    pub surface_inset: Family<Color>,
    pub surface_trough: Family<Color>,
    pub surface_elevated: Family<Color>,
    pub surface_floating: Family<Color>,
    pub surface_inverted: Family<Color>,
    pub neutral: Family<Color>,
}

#[allow(non_upper_case_globals)]
#[rustfmt::skip]
pub mod color {
    use super::{vars, ColorVar, Family, Theme};
    pub const scrim_light: ColorVar = ColorVar::direct(|t: &Theme| t.scrim_light);
    pub const scrim_dark: ColorVar = ColorVar::direct(|t: &Theme| t.scrim_dark);
    pub const error_solid: Family<ColorVar> = vars(|t: &Theme| &t.error_solid);
    pub const error_soft: Family<ColorVar> = vars(|t: &Theme| &t.error_soft);
    pub const info_soft: Family<ColorVar> = vars(|t: &Theme| &t.info_soft);
    pub const info_solid: Family<ColorVar> = vars(|t: &Theme| &t.info_solid);
    pub const primary: Family<ColorVar> = vars(|t: &Theme| &t.primary);
    pub const secondary: Family<ColorVar> = vars(|t: &Theme| &t.secondary);
    pub const success_solid: Family<ColorVar> = vars(|t: &Theme| &t.success_solid);
    pub const success_soft: Family<ColorVar> = vars(|t: &Theme| &t.success_soft);
    pub const surface_canvas: Family<ColorVar> = vars(|t: &Theme| &t.surface_canvas);
    pub const surface_inset: Family<ColorVar> = vars(|t: &Theme| &t.surface_inset);
    pub const surface_trough: Family<ColorVar> = vars(|t: &Theme| &t.surface_trough);
    pub const surface_elevated: Family<ColorVar> = vars(|t: &Theme| &t.surface_elevated);
    pub const surface_floating: Family<ColorVar> = vars(|t: &Theme| &t.surface_floating);
    pub const surface_inverted: Family<ColorVar> = vars(|t: &Theme| &t.surface_inverted);
    pub const neutral: Family<ColorVar> = vars(|t: &Theme| &t.neutral);
}

pub fn default_theme() -> Theme {
    crate::themes::dark::THEME
}
