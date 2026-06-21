use crate::theme::{Family, Theme};
use crate::tokens::palette;

#[rustfmt::skip]
pub const THEME: Theme = Theme {
    scrim_light: palette::SHADE_20,
    scrim_dark: palette::SHADE_60,
    error_solid:      Family { base: palette::CRIMSON_50,  on: palette::SLATE_100,  hover: palette::CRIMSON_40,  active: palette::CRIMSON_30,  border: palette::CRIMSON_60 },
    error_soft:       Family { base: palette::CRIMSON_100, on: palette::CRIMSON_10, hover: palette::CRIMSON_95,  active: palette::CRIMSON_90,  border: palette::CRIMSON_95 },
    info_soft:        Family { base: palette::AZURE_100,   on: palette::AZURE_0,    hover: palette::AZURE_90,    active: palette::AZURE_80,    border: palette::AZURE_30 },
    info_solid:       Family { base: palette::AZURE_20,    on: palette::SLATE_100,  hover: palette::AZURE_90,    active: palette::AZURE_80,    border: palette::AZURE_20 },
    primary:          Family { base: palette::AZURE_50,    on: palette::SLATE_100,  hover: palette::AZURE_40,    active: palette::AZURE_30,    border: palette::AZURE_50 },
    secondary:        Family { base: palette::AZURE_100,   on: palette::AZURE_10,   hover: palette::AZURE_95,    active: palette::AZURE_90,    border: palette::AZURE_100 },
    success_solid:    Family { base: palette::EMERALD_50,  on: palette::SLATE_100,  hover: palette::EMERALD_60,  active: palette::EMERALD_70,  border: palette::EMERALD_50 },
    success_soft:     Family { base: palette::EMERALD_100, on: palette::EMERALD_10, hover: palette::EMERALD_90,  active: palette::EMERALD_80,  border: palette::EMERALD_50 },
    surface_canvas:   Family { base: palette::SLATE_100,   on: palette::SLATE_30,   hover: palette::SLATE_95,    active: palette::SLATE_90,    border: palette::SLATE_65 },
    surface_inset:    Family { base: palette::SLATE_95,    on: palette::SLATE_30,   hover: palette::SLATE_90,    active: palette::SLATE_80,    border: palette::SLATE_80 },
    surface_trough:   Family { base: palette::SLATE_90,    on: palette::SLATE_30,   hover: palette::SLATE_80,    active: palette::SLATE_70,    border: palette::SLATE_60 },
    surface_elevated: Family { base: palette::SLATE_100,   on: palette::SLATE_30,   hover: palette::SLATE_95,    active: palette::SLATE_90,    border: palette::SLATE_65 },
    surface_floating: Family { base: palette::SLATE_100,   on: palette::SLATE_30,   hover: palette::SLATE_95,    active: palette::SLATE_90,    border: palette::SLATE_65 },
    surface_inverted: Family { base: palette::INK_ALPHA_0, on: palette::SLATE_100,  hover: palette::SLATE_20,    active: palette::SLATE_0,     border: palette::INK_ALPHA_0 },
    neutral:          Family { base: palette::SLATE_30,    on: palette::SLATE_90,   hover: palette::SLATE_20,    active: palette::SLATE_0,     border: palette::SLATE_50 },
};
