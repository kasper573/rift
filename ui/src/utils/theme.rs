use std::sync::LazyLock;

use bevy_color::Color;
use bevy_ecs::prelude::Entity;
use bevy_ecs::world::World;
use bevy_view::{Bind, context, provide};

use crate::themes;

/// Handle to one color slot. Carries no color; resolves against active theme at render time.
#[derive(Clone, Copy)]
pub struct ColorVar(fn(&Theme) -> Color);

impl ColorVar {
    pub fn resolve(self, theme: &Theme) -> Color {
        (self.0)(theme)
    }
}

macro_rules! theme_contract {
    ($($slot:ident),+ $(,)?) => {
        #[derive(Clone)]
        #[rustfmt::skip]
        pub struct Theme {
            $(pub $slot: Color,)+
        }

        #[allow(non_upper_case_globals)]
        #[rustfmt::skip]
        pub mod color {
            use super::{ColorVar, Theme};
            $(pub const $slot: ColorVar = ColorVar(|theme: &Theme| theme.$slot);)+
        }
    };
}

theme_contract! {
    scrim_light,
    scrim_dark,

    error_solid_on,
    error_solid_base,
    error_solid_hover,
    error_solid_active,
    error_solid_border,
    error_soft_on,
    error_soft_base,
    error_soft_border,
    error_soft_hover,
    error_soft_active,
    error_soft_on_alt,

    info_soft_on,
    info_soft_base,
    info_soft_hover,
    info_soft_active,
    info_solid_on,
    info_solid_base,
    info_solid_hover,
    info_solid_active,
    info_soft_border,

    primary_base,
    primary_hover,
    primary_active,
    primary_on,

    secondary_on,
    secondary_base,
    secondary_hover,
    secondary_active,
    secondary_on_alt,

    success_solid_base,
    success_solid_hover,
    success_solid_active,
    success_solid_on,
    success_solid_border,
    success_soft_on,
    success_soft_base,
    success_soft_border,
    success_soft_hover,
    success_soft_active,
    success_soft_on_alt,

    surface_canvas_on,
    surface_canvas_on_soft,
    surface_canvas_hover,
    surface_canvas_active,
    surface_inset_on,
    surface_inset_on_soft,
    surface_inset_base,
    surface_inset_hover,
    surface_inset_active,
    surface_trough_on,
    surface_trough_on_soft,
    surface_trough_base,
    surface_trough_hover,
    surface_trough_active,
    surface_elevated_on,
    surface_elevated_on_soft,
    surface_elevated_base,
    surface_elevated_hover,
    surface_elevated_active,
    surface_floating_on,
    surface_floating_on_soft,
    surface_floating_base,
    surface_floating_hover,
    surface_floating_active,
    surface_canvas_base,
    surface_canvas_border,
    surface_inset_border,
    surface_trough_border,
    surface_elevated_border,
    surface_floating_border,
    surface_canvas_border_decorative,
    surface_elevated_border_decorative,
    surface_floating_border_decorative,
    surface_inset_border_decorative,
    surface_trough_border_decorative,
    surface_inverted_on,
    surface_inverted_base,
    surface_inverted_hover,
    surface_inverted_active,

    neutral_on,
    neutral_base,
    neutral_hover,
    neutral_active,
    neutral_border,
}

pub fn provide_theme(theme: Theme) -> Bind {
    provide(theme)
}

/// The [`Theme`] in scope at `entity` — the nearest one [`provide_theme`]d at or above it, or the
/// [default](default_theme) when none is provided (so components render themed without setup).
pub fn active_theme(world: &World, entity: Entity) -> &Theme {
    context::<Theme>(world, entity).unwrap_or_else(|| default_theme())
}

pub fn default_theme() -> &'static Theme {
    static DEFAULT: LazyLock<Theme> = LazyLock::new(themes::dark::theme);
    &DEFAULT
}
