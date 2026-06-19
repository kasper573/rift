//! The shared design tokens.
//! the raw [`palette`] of colors plus the unitless [`size`], [`spacing`] and [`radius`] scales
//! (all logical pixels) and the [`font`] families and weights. These are theme-independent —
//! the per-theme semantic colors that *reference* this palette live in [`crate::themes`].

/// The raw color palette every theme draws its semantic colors from.
#[rustfmt::skip]
pub mod palette {
    use bevy_color::Color;

    pub const AZURE_100: Color = Color::srgb_u8(231, 239, 252);
    pub const AZURE_95: Color = Color::srgb_u8(185, 217, 250);
    pub const AZURE_90: Color = Color::srgb_u8(147, 190, 241);
    pub const AZURE_80: Color = Color::srgb_u8(104, 168, 235);
    pub const AZURE_70: Color = Color::srgb_u8(68, 140, 225);
    pub const AZURE_60: Color = Color::srgb_u8(27, 118, 215);
    pub const AZURE_50: Color = Color::srgb_u8(22, 98, 200);
    pub const AZURE_40: Color = Color::srgb_u8(11, 85, 183);
    pub const AZURE_30: Color = Color::srgb_u8(7, 66, 167);
    pub const AZURE_20: Color = Color::srgb_u8(3, 52, 147);
    pub const AZURE_10: Color = Color::srgb_u8(1, 40, 122);
    pub const AZURE_0: Color = Color::srgb_u8(2, 27, 104);
    pub const AZURE_ALPHA_25: Color = Color::srgba_u8(60, 156, 232, 64);
    pub const AZURE_ALPHA_50: Color = Color::srgba_u8(27, 118, 215, 128);
    pub const AZURE_ALPHA_75: Color = Color::srgba_u8(27, 118, 215, 191);
    pub const AZURE_ALPHA_100: Color = Color::srgb_u8(27, 118, 215);

    pub const INK_ALPHA_100: Color = Color::srgba_u8(16, 20, 20, 3);
    pub const INK_ALPHA_95: Color = Color::srgba_u8(16, 20, 20, 13);
    pub const INK_ALPHA_90: Color = Color::srgba_u8(16, 20, 20, 26);
    pub const INK_ALPHA_80: Color = Color::srgba_u8(16, 20, 20, 51);
    pub const INK_ALPHA_70: Color = Color::srgba_u8(16, 20, 20, 77);
    pub const INK_ALPHA_65: Color = Color::srgba_u8(16, 20, 20, 102);
    pub const INK_ALPHA_60: Color = Color::srgba_u8(16, 20, 20, 128);
    pub const INK_ALPHA_50: Color = Color::srgba_u8(16, 20, 20, 153);
    pub const INK_ALPHA_40: Color = Color::srgba_u8(16, 20, 20, 179);
    pub const INK_ALPHA_30: Color = Color::srgba_u8(16, 20, 20, 204);
    pub const INK_ALPHA_20: Color = Color::srgba_u8(16, 20, 20, 230);
    pub const INK_ALPHA_10: Color = Color::srgba_u8(16, 20, 20, 242);
    pub const INK_ALPHA_0: Color = Color::srgb_u8(16, 20, 20);

    pub const SHADE_100: Color = Color::srgb_u8(0, 0, 0);
    pub const SHADE_60: Color = Color::srgba_u8(0, 0, 0, 153);
    pub const SHADE_40: Color = Color::srgba_u8(0, 0, 0, 102);
    pub const SHADE_20: Color = Color::srgba_u8(0, 0, 0, 51);
    pub const SHADE_0: Color = Color::srgba_u8(0, 0, 0, 0);
    pub const SHADE_80: Color = Color::srgba_u8(0, 0, 0, 204);

    pub const EMERALD_100: Color = Color::srgb_u8(230, 245, 233);
    pub const EMERALD_95: Color = Color::srgb_u8(193, 223, 190);
    pub const EMERALD_90: Color = Color::srgb_u8(150, 205, 151);
    pub const EMERALD_80: Color = Color::srgb_u8(114, 180, 111);
    pub const EMERALD_70: Color = Color::srgb_u8(72, 160, 74);
    pub const EMERALD_60: Color = Color::srgb_u8(36, 134, 39);
    pub const EMERALD_50: Color = Color::srgb_u8(28, 127, 32);
    pub const EMERALD_40: Color = Color::srgb_u8(25, 112, 28);
    pub const EMERALD_30: Color = Color::srgb_u8(17, 104, 27);
    pub const EMERALD_20: Color = Color::srgb_u8(15, 88, 21);
    pub const EMERALD_10: Color = Color::srgb_u8(8, 78, 22);
    pub const EMERALD_0: Color = Color::srgb_u8(7, 61, 19);
    pub const EMERALD_ALPHA_25: Color = Color::srgba_u8(36, 134, 39, 64);
    pub const EMERALD_ALPHA_50: Color = Color::srgba_u8(36, 134, 39, 128);
    pub const EMERALD_ALPHA_75: Color = Color::srgba_u8(36, 134, 39, 191);
    pub const EMERALD_ALPHA_100: Color = Color::srgb_u8(36, 134, 39);

    pub const SLATE_100: Color = Color::srgb_u8(255, 255, 255);
    pub const SLATE_95: Color = Color::srgb_u8(244, 243, 250);
    pub const SLATE_90: Color = Color::srgb_u8(228, 234, 239);
    pub const SLATE_80: Color = Color::srgb_u8(187, 192, 199);
    pub const SLATE_70: Color = Color::srgb_u8(159, 168, 176);
    pub const SLATE_60: Color = Color::srgb_u8(113, 121, 125);
    pub const SLATE_50: Color = Color::srgb_u8(73, 74, 80);
    pub const SLATE_40: Color = Color::srgb_u8(41, 46, 45);
    pub const SLATE_30: Color = Color::srgb_u8(33, 32, 38);
    pub const SLATE_20: Color = Color::srgb_u8(23, 28, 28);
    pub const SLATE_10: Color = Color::srgb_u8(16, 20, 20);
    pub const SLATE_0: Color = Color::srgb_u8(0, 0, 0);
    pub const SLATE_65: Color = Color::srgb_u8(144, 148, 155);

    pub const CRIMSON_100: Color = Color::srgb_u8(251, 235, 232);
    pub const CRIMSON_95: Color = Color::srgb_u8(248, 192, 191);
    pub const CRIMSON_90: Color = Color::srgb_u8(238, 157, 153);
    pub const CRIMSON_80: Color = Color::srgb_u8(233, 117, 120);
    pub const CRIMSON_70: Color = Color::srgb_u8(221, 84, 80);
    pub const CRIMSON_60: Color = Color::srgb_u8(213, 46, 45);
    pub const CRIMSON_50: Color = Color::srgb_u8(194, 44, 39);
    pub const CRIMSON_40: Color = Color::srgb_u8(180, 36, 27);
    pub const CRIMSON_30: Color = Color::srgb_u8(158, 35, 23);
    pub const CRIMSON_20: Color = Color::srgb_u8(142, 28, 13);
    pub const CRIMSON_10: Color = Color::srgb_u8(118, 27, 12);
    pub const CRIMSON_0: Color = Color::srgb_u8(100, 20, 8);
}

/// The numeric size scale the other scales are derived from (logical px).
pub mod size {
    pub const STEP_100: f32 = 4.0;
    pub const STEP_200: f32 = 8.0;
    pub const STEP_400: f32 = 16.0;
    pub const STEP_550: f32 = 22.0;
    pub const STEP_600: f32 = 24.0;
    pub const STEP_1000: f32 = 40.0;
}

/// Spacing between elements (logical px).
pub mod spacing {
    pub const S: f32 = 2.0;
    pub const M: f32 = 4.0;
    pub const L: f32 = 8.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 24.0;
    pub const XXXL: f32 = 40.0;
}

/// Corner radii (logical px); `PILL` is an effectively-pill value.
pub mod radius {
    pub const PILL: f32 = 999.0;
    pub const S: f32 = 4.0;
    pub const M: f32 = 8.0;
    pub const L: f32 = 16.0;
}

/// Font families and weights. Identical across themes, so they live here rather than in a theme.
pub mod font {
    pub const FAMILY_TEXT: &str = "Lato";
    pub const FAMILY_DISPLAY: &str = "Circular TT";
    pub const WEIGHT_REGULAR: u16 = 400;
    pub const WEIGHT_MEDIUM: u16 = 500;
    pub const WEIGHT_BOLD: u16 = 700;
}

/// The typography scale — one entry per text intent, carrying the font size, line height and weight
/// paired with that role. These are theme-independent (identical in light and dark), so they live here
/// rather than in a per-theme contract.
pub mod typography {
    use super::{font, size};

    /// A resolved typographic style: the four values that distinguish one text intent from another —
    /// `family` is the font family name (matched against the registered TTFs).
    #[derive(Clone, Copy)]
    pub struct Typography {
        pub font_size: f32,
        pub line_height: f32,
        pub weight: u16,
        pub family: &'static str,
    }

    const fn t(font_size: f32, line_height: f32, weight: u16, family: &'static str) -> Typography {
        Typography {
            font_size,
            line_height,
            weight,
            family,
        }
    }

    pub const BODY: Typography = t(
        size::STEP_400,
        size::STEP_550,
        font::WEIGHT_REGULAR,
        font::FAMILY_TEXT,
    );

    pub fn by_name(_name: &str) -> Typography {
        // TODO match on name when we have more than one typography
        // match name {
        //     _ => BODY,
        // }
        BODY
    }
}
