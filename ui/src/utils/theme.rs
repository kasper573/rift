//! Theme variables, a `bevy_ui` adaptation of [vanilla-extract's `createThemeContract`](https://vanilla-extract.style/documentation/api/create-theme-contract/)
//! and [`createTheme`](https://vanilla-extract.style/documentation/api/create-theme/).
//!
//! [`Theme`] is the **contract**: a struct with one [`Color`] field per semantic slot a component can
//! reference. A concrete theme (see [`crate::themes`]) *creates* a theme by filling every field — and
//! because it's a struct literal, the compiler enforces completeness, a stronger guarantee than
//! vanilla-extract's runtime check. The [`color`] module exposes one [`ColorVar`] handle per slot —
//! the analog of vanilla-extract's `vars.color.*` — which recipes reference instead of literal colors.
//!
//! There are no class names: the active theme is supplied through [`bevy_view`] context with
//! [`provide_theme`], and a [`ColorVar`] resolves against the nearest provided theme at render time
//! (falling back to a default). Because recipe styling re-applies every frame, swapping the provided
//! theme recolors the subtree on the next frame with no extra machinery.

use std::sync::LazyLock;

use bevy_color::Color;
use bevy_ecs::prelude::Entity;
use bevy_ecs::world::World;
use bevy_view::{Bind, context, provide};

use crate::themes;

/// A handle to one color slot of the [`Theme`] contract — the analog of a vanilla-extract theme var.
/// It carries no color itself; it resolves against whichever [`Theme`] is active where it's used.
#[derive(Clone, Copy)]
pub struct ColorVar(fn(&Theme) -> Color);

impl ColorVar {
    /// The color this var takes in `theme`.
    pub fn resolve(self, theme: &Theme) -> Color {
        (self.0)(theme)
    }
}

/// Generates the [`Theme`] contract struct (one [`Color`] field per slot) and the [`color`] module of
/// matching [`ColorVar`] handles from a single slot list — so the contract is declared once and a var
/// can't drift from the struct.
macro_rules! theme_contract {
    ($($slot:ident),+ $(,)?) => {
        /// The theme contract: every semantic color slot the components can reference. Create a theme
        /// by constructing this with each field mapped to a palette color (see [`crate::themes`]); the
        /// compiler enforces that every slot is filled.
        #[derive(Clone)]
        #[rustfmt::skip]
        pub struct Theme {
            $(pub $slot: Color,)+
        }

        /// The theme variables — one [`ColorVar`] per contract slot, the analog of vanilla-extract's
        /// `vars.color.*`. Recipes reference these (e.g. `color::primary_base`); they resolve against
        /// the active theme at render time.
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

/// Supplies `theme` to the subtree it decorates, via [`bevy_view`] context. Apply it to a root element
/// with `use={provide_theme(themes::light::theme())}`; every recipe below resolves its [`ColorVar`]s
/// against it. An inner provider re-themes its own subtree.
pub fn provide_theme(theme: Theme) -> Bind {
    provide(theme)
}

/// The [`Theme`] in scope at `entity` — the nearest one [`provide_theme`]d at or above it, or the
/// [default](default_theme) when none is provided (so components render themed without setup).
pub fn active_theme(world: &World, entity: Entity) -> &Theme {
    context::<Theme>(world, entity).unwrap_or_else(|| default_theme())
}

/// The theme used when no [`provide_theme`] is in scope: the dark theme, built once.
pub fn default_theme() -> &'static Theme {
    static DEFAULT: LazyLock<Theme> = LazyLock::new(themes::dark::theme);
    &DEFAULT
}
