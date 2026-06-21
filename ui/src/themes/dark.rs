use crate::theme::{Family, Theme};
use crate::tokens::palette;

#[rustfmt::skip]
pub const THEME: Theme = Theme {
    scrim_light: palette::SHADE_20,
    scrim_dark: palette::SHADE_60,
    error_solid:      Family { base: palette::CRIMSON_60,  on: palette::SLATE_100,  hover: palette::CRIMSON_50,  active: palette::CRIMSON_40,  border: palette::CRIMSON_70 },
    error_soft:       Family { base: palette::CRIMSON_100, on: palette::CRIMSON_10, hover: palette::CRIMSON_80,  active: palette::CRIMSON_90,  border: palette::CRIMSON_10 },
    info_soft:        Family { base: palette::AZURE_95,    on: palette::AZURE_50,   hover: palette::AZURE_90,    active: palette::AZURE_80,    border: palette::AZURE_30 },
    info_solid:       Family { base: palette::AZURE_90,    on: palette::AZURE_60,   hover: palette::AZURE_80,    active: palette::AZURE_70,    border: palette::AZURE_90 },
    primary:          Family { base: palette::AZURE_40,    on: palette::SLATE_100,  hover: palette::AZURE_30,    active: palette::AZURE_20,    border: palette::AZURE_40 },
    secondary:        Family { base: palette::AZURE_90,    on: palette::AZURE_60,   hover: palette::AZURE_80,    active: palette::AZURE_70,    border: palette::AZURE_90 },
    success_solid:    Family { base: palette::EMERALD_50,  on: palette::SLATE_100,  hover: palette::EMERALD_60,  active: palette::EMERALD_70,  border: palette::EMERALD_50 },
    success_soft:     Family { base: palette::EMERALD_95,  on: palette::EMERALD_10, hover: palette::EMERALD_90,  active: palette::EMERALD_80,  border: palette::EMERALD_50 },
    surface_canvas:   Family { base: palette::SLATE_30,    on: palette::SLATE_90,   hover: palette::SLATE_20,    active: palette::SLATE_10,    border: palette::SLATE_50 },
    surface_inset:    Family { base: palette::SLATE_20,    on: palette::SLATE_90,   hover: palette::SLATE_20,    active: palette::SLATE_0,     border: palette::SLATE_20 },
    surface_trough:   Family { base: palette::SLATE_10,    on: palette::SLATE_100,  hover: palette::SLATE_0,     active: palette::SLATE_20,    border: palette::SLATE_10 },
    surface_elevated: Family { base: palette::SLATE_40,    on: palette::SLATE_90,   hover: palette::SLATE_30,    active: palette::SLATE_20,    border: palette::SLATE_50 },
    surface_floating: Family { base: palette::SLATE_50,    on: palette::SLATE_90,   hover: palette::SLATE_40,    active: palette::SLATE_30,    border: palette::SLATE_20 },
    surface_inverted: Family { base: palette::SLATE_100,   on: palette::SLATE_30,   hover: palette::SLATE_95,    active: palette::SLATE_90,    border: palette::SLATE_100 },
    neutral:          Family { base: palette::SLATE_30,    on: palette::SLATE_90,   hover: palette::SLATE_20,    active: palette::SLATE_0,     border: palette::SLATE_50 },
};
